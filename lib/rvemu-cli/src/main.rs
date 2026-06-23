use clap::{App, Arg};
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::iter::FromIterator;

use rvemu_core::bus::DRAM_BASE;
use rvemu_core::cpu::Cpu;
use rvemu_core::emulator::Emulator;

// Offset standard utilisé par OpenSBI (FW_JUMP_OFFSET) pour sauter sur le
// noyau : DRAM_BASE + 0x200000. Si ton fw_jump.bin a été compilé avec un
// FW_JUMP_OFFSET différent, ajuste cette constante en conséquence.
const KERNEL_OFFSET: u64 = 0x20_0000;

// ── Terminal raw mode ────────────────────────────────────────────────────────
// (inchangé)
#[cfg(windows)]
mod terminal {
    use windows_sys::Win32::System::Console::*;

    pub fn set_raw() -> u32 {
        unsafe {
            let handle = GetStdHandle(STD_INPUT_HANDLE);
            let mut mode = 0u32;
            GetConsoleMode(handle, &mut mode);
            let new_mode = (mode
                & !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT))
                | ENABLE_VIRTUAL_TERMINAL_INPUT;
            SetConsoleMode(handle, new_mode);
            mode
        }
    }

    pub fn restore(old_mode: u32) {
        unsafe {
            let handle = GetStdHandle(STD_INPUT_HANDLE);
            SetConsoleMode(handle, old_mode);
        }
    }
}

#[cfg(unix)]
mod terminal {
    pub fn set_raw() -> u32 { 0 }
    pub fn restore(_: u32) {}
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn dump_registers(cpu: &Cpu) {
    println!("-------------------------------------------------------------------------------------------");
    println!("{}", cpu.xregs);
    println!("-------------------------------------------------------------------------------------------");
    println!("{}", cpu.fregs);
    println!("-------------------------------------------------------------------------------------------");
    println!("{}", cpu.state);
    println!("-------------------------------------------------------------------------------------------");
    println!("pc: {:#x}", cpu.pc);
}

fn dump_count(cpu: &Cpu) {
    if cpu.is_count {
        println!("===========================================================================================");
        let mut sorted_counter = Vec::from_iter(&cpu.inst_counter);
        sorted_counter.sort_by(|&(_, a), &(_, b)| b.cmp(&a));
        for (inst, count) in sorted_counter.iter() {
            println!("{}, {}", inst, count);
        }
        println!("===========================================================================================");
    }
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() -> io::Result<()> {
    let matches = App::new("rvemu: RISC-V emulator")
        .version("0.0.1")
        .author("Asami Doi <@d0iasm>")
        .arg(Arg::with_name("kernel").short("k").long("kernel").takes_value(true).required(true))
        .arg(
            Arg::with_name("firmware")
                .long("firmware")
                .takes_value(true)
                .help("M-mode firmware (e.g. OpenSBI fw_jump.bin), loaded at DRAM_BASE"),
        )
        .arg(Arg::with_name("file").short("f").long("file").takes_value(true))
        .arg(Arg::with_name("debug").short("d").long("debug"))
        .arg(Arg::with_name("count").short("c").long("count"))
        .get_matches();

    let mut kernel_data = Vec::new();
    File::open(matches.value_of("kernel").unwrap())?.read_to_end(&mut kernel_data)?;

    let mut img_data = Vec::new();
    if let Some(f) = matches.value_of("file") {
        File::open(f)?.read_to_end(&mut img_data)?;
    }

    let mut emu = Emulator::new();

    match matches.value_of("firmware") {
        Some(fw_path) => {
            // Boot M-mode : firmware à DRAM_BASE, kernel à DRAM_BASE + KERNEL_OFFSET.
            let mut fw_data = Vec::new();
            File::open(fw_path)?.read_to_end(&mut fw_data)?;

            if fw_data.len() as u64 > KERNEL_OFFSET {
                panic!(
                    "firmware ({} bytes) déborde sur KERNEL_OFFSET ({} bytes) ; \
                     augmente KERNEL_OFFSET ou vérifie le FW_JUMP_OFFSET réel d'OpenSBI",
                    fw_data.len(),
                    KERNEL_OFFSET
                );
            }

            emu.initialize_dram_at(fw_data, 0);
            emu.initialize_dram_at(kernel_data, KERNEL_OFFSET);
            emu.initialize_pc(DRAM_BASE); // le CPU démarre sur le firmware, en M-mode
        }
        None => {
            // Pas de firmware : kernel chargé directement à DRAM_BASE (ancien comportement).
            emu.initialize_dram_at(kernel_data, 0);
            emu.initialize_pc(DRAM_BASE);
        }
    }

    const DTB_ADDR: u64 = 0x1020;
    emu.cpu.xregs.write(11, DTB_ADDR); // a1 = adresse du DTB
    emu.cpu.xregs.write(10, 0);

    emu.initialize_disk(img_data);
    emu.is_debug = matches.occurrences_of("debug") == 1;
    emu.cpu.is_count = matches.occurrences_of("count") == 1;

    let old_mode = terminal::set_raw();
    emu.start();
    terminal::restore(old_mode);

    dump_registers(&emu.cpu);
    dump_count(&emu.cpu);

    Ok(())
}