//! The uart module contains the implementation of a universal asynchronous receiver-transmitter
//! (UART) for the CLI tool. The device is 16550A UART, which is used in the QEMU virt machine.
//! See more information in http://byterunner.com/16550.html.

use std::io;
use std::io::prelude::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex,
};
use std::thread;

use crate::bus::{UART_BASE, UART_SIZE};
use crate::cpu::BYTE;
use crate::exception::Exception;

/// The interrupt request of UART.
pub const UART_IRQ: u64 = 10;

/// Receive holding register (for input bytes).
const UART_RHR: u64 = UART_BASE + 0;
/// Transmit holding register (for output bytes).
const UART_THR: u64 = UART_BASE + 0;
/// Interrupt enable register.
/// FIFO control register.
const _UART_FCR: u64 = UART_BASE + 2;


/// Interrupt status register.
/// ISR BIT-0:
///     0 = an interrupt is pending and the ISR contents may be used as a pointer to the appropriate
/// interrupt service routine.
///     1 = no interrupt is pending.
const _UART_ISR: u64 = UART_BASE + 2;
/// Line control register.
const _UART_LCR: u64 = UART_BASE + 3;
/// Line status register.
/// LSR BIT 0:
///     0 = no data in receive holding register or FIFO.
///     1 = data has been receive and saved in the receive holding register or FIFO.
/// LSR BIT 5:
///     0 = transmit holding register is full. 16550 will not accept any data for transmission.
///     1 = transmitter hold register (or FIFO) is empty. CPU can load the next character.
const UART_LSR: u64 = UART_BASE + 5;

/// The receiver (RX).
const UART_LSR_RX: u8 = 1;
/// The transmitter (TX).
const UART_LSR_TX: u8 = 1 << 5;

/// Interrupt enable register.
const UART_IER: u64 = UART_BASE + 1;
/// Interrupt identification register (read) / FIFO control register (write, same offset).
const UART_IIR: u64 = UART_BASE + 2;

/// Bit 1 of IER: Enable Transmitter Holding Register Empty Interrupt.
const UART_IER_THRI: u8 = 1 << 1;
/// Bit 0 of IER: Enable Received Data Available Interrupt.
const UART_IER_RDI: u8 = 1 << 0;

/// IIR encoding (bits 3:1, with bit 0 = 0 meaning "interrupt pending"):
/// 0x06 = Receiver Line Status
/// 0x04 = Received Data Available
/// 0x02 = Transmitter Holding Register Empty
/// 0x00 = Modem Status
/// When no interrupt is pending, IIR = 0x01 (bit 0 set).
const UART_IIR_NONE: u8 = 0x01;
const UART_IIR_THRE: u8 = 0x02;
const UART_IIR_RDA: u8 = 0x04;


/// The UART, the size of which is 0x100 (2**8).
pub struct Uart {
    uart: Arc<(Mutex<[u8; UART_SIZE as usize]>, Condvar)>,
    interrupting: Arc<AtomicBool>,
}

impl Uart {
    /// Create a new UART object.
    pub fn new() -> Self {

        let uart = Arc::new((Mutex::new([0; UART_SIZE as usize]), Condvar::new()));
        let interrupting = Arc::new(AtomicBool::new(false));
        {
            let (uart, _cvar) = &*uart;
            let mut uart = uart.lock().expect("failed to get an UART object");
            // Transmitter hold register is empty. It allows input anytime.
            uart[(UART_LSR - UART_BASE) as usize] |= UART_LSR_TX;
        }

        // Create a new thread for waiting for input.
        let mut byte = [0; 1];
        let cloned_uart = uart.clone();
        let cloned_interrupting = interrupting.clone();
        let _uart_thread_for_read = thread::spawn(move || loop {
            match io::stdin().read(&mut byte) {
                Ok(_) => {
                    let (uart, cvar) = &*cloned_uart;
                    let mut uart = uart.lock().expect("failed to get an UART object");
                    // Wait for the thread to start up.
                    while (uart[(UART_LSR - UART_BASE) as usize] & UART_LSR_RX) == 1 {
                        uart = cvar.wait(uart).expect("the mutex is poisoned");
                    }
                    uart[0] = byte[0];
                    cloned_interrupting.store(true, Ordering::Release);
                    // Data has been receive.
                    uart[(UART_LSR - UART_BASE) as usize] |= UART_LSR_RX;
                }
                Err(e) => {
                    println!("input via UART is error: {}", e);
                }
            }
        });

        Self { uart, interrupting }
    }

    pub fn new_headless() -> Self {
        Self::new_with_stdin(false)
    }

    fn new_with_stdin(enable: bool) -> Self {
        let uart = Arc::new((Mutex::new([0; UART_SIZE as usize]), Condvar::new()));
        let interrupting = Arc::new(AtomicBool::new(false));

        // init LSR...
        {
            let (uart_lock, _) = &*uart;
            let mut u = uart_lock.lock().unwrap();
            u[(UART_LSR - UART_BASE) as usize] |= UART_LSR_TX;
        }

        if enable {
            let mut byte = [0; 1];
            let _cloned_uart = uart.clone();
            let _cloned_interrupting = interrupting.clone();
            thread::spawn(move || loop {
                match io::stdin().read(&mut byte) {
                    Ok(_) => { /* ... code existant ... */ }
                    Err(e) => { println!("input via UART is error: {}", e); }
                }
            });
        }

        Self { uart, interrupting }
    }

    /// Return true if an interrupt is pending. Clear the interrupting flag by swapping a value.
    pub fn is_interrupting(&self) -> bool {
        self.interrupting.swap(false, Ordering::Acquire)
    }

    /// Read a byte from the receive holding register.
    pub fn read(&mut self, index: u64, size: u8) -> Result<u64, Exception> {
        if size != BYTE {
            return Err(Exception::LoadAccessFault);
        }

        let (uart, cvar) = &*self.uart;
        let mut uart = uart.lock().expect("failed to get an UART object");
        match index {
            UART_RHR => {
                cvar.notify_one();
                uart[(UART_LSR - UART_BASE) as usize] &= !UART_LSR_RX;
                Ok(uart[(UART_RHR - UART_BASE) as usize] as u64)
            }
            UART_IIR => {
                let ier = uart[(UART_IER - UART_BASE) as usize];
                let lsr = uart[(UART_LSR - UART_BASE) as usize];

                // Priority order (highest first): RX data available, then THR empty.
                let iir = if (ier & UART_IER_RDI != 0) && (lsr & UART_LSR_RX != 0) {
                    UART_IIR_RDA
                } else if (ier & UART_IER_THRI != 0) && (lsr & UART_LSR_TX != 0) {
                    UART_IIR_THRE
                } else {
                    UART_IIR_NONE
                };

                // Reading IIR clears a pending THRE interrupt indication on real hardware.
                Ok(iir as u64)
            }
            _ => Ok(uart[(index - UART_BASE) as usize] as u64),
        }
    }

    /// Write a byte to the transmit holding register.
    pub fn write(&mut self, index: u64, value: u8, size: u8) -> Result<(), Exception> {
        if size != BYTE {
            return Err(Exception::StoreAMOAccessFault);
        }

        let (uart, _cvar) = &*self.uart;
        let mut uart = uart.lock().expect("failed to get an UART object");
        match index {
            UART_THR => {
                print!("{}", value as char);
                io::stdout().flush().expect("failed to flush stdout");

                let ier = uart[(UART_IER - UART_BASE) as usize];
                if ier & UART_IER_THRI != 0 {
                    self.interrupting.store(true, Ordering::Release);
                }
            }
            UART_IER => {
                uart[(index - UART_BASE) as usize] = value;
                let lsr = uart[(UART_LSR - UART_BASE) as usize];
                if value & UART_IER_THRI != 0 && lsr & UART_LSR_TX != 0 {
                    self.interrupting.store(true, Ordering::Release);
                }
            }
            _ => {
                uart[(index - UART_BASE) as usize] = value;
            }
        }
        Ok(())
    }
}
