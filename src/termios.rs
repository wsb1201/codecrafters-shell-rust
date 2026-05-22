#![allow(unused)]

use std::{ffi::*, io, mem::MaybeUninit, os::unix::prelude::*};

pub use libc::{
	B0, B50, B75, B110, B134, B150, B200, B300, B600, B1200, B1800, B2400, B4800, B9600, B19200,
	B38400, BRKINT, BS0, BS1, BSDLY, CLOCAL, CR0, CR1, CR2, CR3, CRDLY, CREAD, CS5, CS6, CS7, CS8,
	CSIZE, CSTOPB, ECHO, ECHOCTL, ECHOE, ECHOK, ECHONL, FF0, FF1, FFDLY, HUPCL, ICANON, ICRNL,
	IEXTEN, IGNBRK, IGNCR, IGNPAR, INLCR, INPCK, ISIG, ISTRIP, IXANY, IXOFF, IXON, NL0, NL1, NLDLY,
	NOFLSH, OCRNL, OFDEL, OFILL, ONLCR, ONLRET, ONOCR, OPOST, PARENB, PARMRK, PARODD, TAB0, TAB1,
	TAB2, TAB3, TABDLY, TCIFLUSH, TCIOFF, TCIOFLUSH, TCION, TCOFLUSH, TCOOFF, TCOON, TCSADRAIN,
	TCSAFLUSH, TCSANOW, TOSTOP, VEOF, VEOL, VERASE, VINTR, VKILL, VMIN, VQUIT, VSTART, VSTOP,
	VSUSP, VT0, VT1, VTDLY, VTIME,
};

pub fn tcgetattr(fd: impl AsRawFd) -> io::Result<libc::termios> {
	let mut termios = MaybeUninit::uninit();
	match unsafe { libc::tcgetattr(fd.as_raw_fd(), termios.as_mut_ptr()) } {
		0 => Ok(unsafe { termios.assume_init() }),
		-1 => Err(io::Error::last_os_error()),
		_ => unreachable!(),
	}
}

pub fn tcsetattr(
	fd: impl AsRawFd,
	optional_actions: c_int,
	termios: &libc::termios,
) -> io::Result<()> {
	match unsafe { libc::tcsetattr(fd.as_raw_fd(), optional_actions, termios) } {
		0 => Ok(()),
		-1 => Err(io::Error::last_os_error()),
		_ => unreachable!(),
	}
}

pub fn cfmakeraw(termios: &mut libc::termios) {
	unsafe { libc::cfmakeraw(termios) }
}

pub unsafe fn cfgetispeed(termios: *const libc::termios) -> libc::speed_t {
	unsafe { libc::cfgetispeed(termios) }
}

pub unsafe fn cfgetospeed(termios: *const libc::termios) -> libc::speed_t {
	unsafe { libc::cfgetospeed(termios) }
}

pub unsafe fn cfsetispeed(termios: *mut libc::termios, speed: libc::speed_t) -> c_int {
	unsafe { libc::cfsetispeed(termios, speed) }
}

pub unsafe fn cfsetospeed(termios: *mut libc::termios, speed: libc::speed_t) -> c_int {
	unsafe { libc::cfsetospeed(termios, speed) }
}

pub unsafe fn cfsetspeed(termios: *mut libc::termios, speed: libc::speed_t) -> c_int {
	unsafe { libc::cfsetspeed(termios, speed) }
}

pub unsafe fn tcdrain(fd: impl AsRawFd) -> c_int {
	unsafe { libc::tcdrain(fd.as_raw_fd()) }
}

pub unsafe fn tcflow(fd: impl AsRawFd, action: c_int) -> c_int {
	unsafe { libc::tcflow(fd.as_raw_fd(), action) }
}

pub unsafe fn tcflush(fd: impl AsRawFd, action: c_int) -> c_int {
	unsafe { libc::tcflush(fd.as_raw_fd(), action) }
}

pub unsafe fn tcgetsid(fd: impl AsRawFd) -> libc::pid_t {
	unsafe { libc::tcgetsid(fd.as_raw_fd()) }
}

pub unsafe fn tcsendbreak(fd: impl AsRawFd, duration: c_int) -> c_int {
	unsafe { libc::tcsendbreak(fd.as_raw_fd(), duration) }
}
