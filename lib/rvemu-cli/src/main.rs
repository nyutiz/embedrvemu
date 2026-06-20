use clap::{App, Arg};
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::iter::FromIterator;

use rvemu_core::bus::DRAM_BASE;
use rvemu_core::cpu::Cpu;
use rvemu_core::emulator::Emulator;

// ── Terminal raw mode ────────────────────────────────────────────────────────

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
            mode // retourne l'ancien mode pour restauration
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
    emu.initialize_dram(kernel_data);
    emu.initialize_disk(img_data);
    emu.initialize_pc(DRAM_BASE);
    emu.is_debug = matches.occurrences_of("debug") == 1;
    emu.cpu.is_count = matches.occurrences_of("count") == 1;

    let old_mode = terminal::set_raw();
    emu.start();
    terminal::restore(old_mode);

    dump_registers(&emu.cpu);
    dump_count(&emu.cpu);

    Ok(())
}