use msp430::{critical_section, interrupt};

use super::peripherals::{UCSI, UCTXIFG};
use core::fmt::{self, Write};

use super::peripherals;

static mut HOST_OSTREAM: Option<HostStream> = None;

struct HostStream {
    ucsi: UCSI,
}

fn init_host_ostream() {
    unsafe {
        HOST_OSTREAM = Some(HostStream::new());
    }
}

impl HostStream {
    pub fn new() -> Self {
        // default to use UCSIA0
        Self {
            ucsi: UCSI::new(peripherals::USCIPort::EUSCIA0),
        }
    }
}

impl HostStream {
    pub fn write_all(&self, s: &str) -> fmt::Result {
        write_all(&self, s.as_bytes()).map_err(|_| fmt::Error)
    }
}

impl fmt::Write for HostStream {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.write_all(s)
    }
}

fn write_all(hs: &HostStream, buffer: &[u8]) -> Result<(), ()> {
    for c in buffer {
        while hs.ucsi.p.ifg.read() & UCTXIFG == 0 {
            // wait
        }
        unsafe {
            hs.ucsi.p.txbuf.write(*c as u16);
        }
    }
    Ok(())
}

pub fn hstdout_str(s: &str) {
    let _result = critical_section::with(|cs| unsafe {
        if HOST_OSTREAM.is_none() {
            init_host_ostream();
        }
        HOST_OSTREAM.as_mut().unwrap().write_str(s).map_err(drop)
    });
}

pub fn hstdout_fmt(args: fmt::Arguments) {
    let _result = critical_section::with(|cs| unsafe {
        if HOST_OSTREAM.is_none() {
            init_host_ostream();
        }
        HOST_OSTREAM.as_mut().unwrap().write_fmt(args).map_err(drop)
    });
}

macro_rules! hprint {
    ($s:expr) => {
        hstdout_str($s)
    };
    ($($tt:tt)*) => {
        hstdout_fmt(format_args!($($tt)*))
    };
}

macro_rules! hprintln {
    () => {
        hstdout_str("\n\r")
    };
    ($s:expr) => {
        hstdout_str(concat!($s, "\n\r"))
    };
    ($s:expr, $($tt:tt)*) => {
        hstdout_fmt(format_args!(concat!($s, "\n\r"), $($tt)*))
    };
}

pub(in crate::board) fn msp430_hprintln(args: core::fmt::Arguments) {
    hprintln!("{}", args);
}

pub(in crate::board) fn msp430_hprint(args: core::fmt::Arguments) {
    hprint!("{}", args);
}
