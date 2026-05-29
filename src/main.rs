#![allow(unused_imports)]

use std::{
	borrow::Cow,
	cell::{Cell, LazyCell},
	env,
	ffi::OsStr,
	fmt, fs,
	io::{self, BufRead, IsTerminal, Read, Write},
	mem,
	os::unix::prelude::*,
	path::{Path, PathBuf},
	process,
};

mod interactive;
mod termios;

struct TermiosMode<'a> {
	fd: BorrowedFd<'a>,
	termios_orig: libc::termios,
	termios_repl: libc::termios,
}

impl<'a> TermiosMode<'a> {
	fn new(fd: BorrowedFd<'a>) -> io::Result<Self> {
		let termios_orig = termios::tcgetattr(fd)?;

		let mut t = termios_orig.clone();
		t.c_lflag &= !(termios::ICANON | termios::ECHO | termios::ECHOCTL);
		t.c_lflag |= termios::ISIG | termios::IEXTEN;
		t.c_iflag |= termios::ICRNL;
		t.c_oflag |= termios::OPOST;
		t.c_cc[termios::VMIN] = 1;
		t.c_cc[termios::VTIME] = 0;
		termios::tcsetattr(fd, termios::TCSANOW, &t)?;

		Ok(Self { fd, termios_orig, termios_repl: t })
	}

	fn set_repl_mode(&self) -> io::Result<()> {
		termios::tcsetattr(self.fd, termios::TCSADRAIN, &self.termios_repl)
	}

	fn restore_orig(&self) -> io::Result<()> {
		termios::tcsetattr(self.fd, termios::TCSADRAIN, &self.termios_orig)
	}
}

impl Drop for TermiosMode<'_> {
	fn drop(&mut self) {
		_ = self.restore_orig();
	}
}

fn main() -> io::Result<()> {
	let (i, mut o, mut e) = (io::stdin(), io::stdout(), io::stderr());
	let termios_mode = i
		.is_terminal()
		.then(|| TermiosMode::new(i.as_fd()))
		.transpose()?;

	loop {
		interactive::next()?;
	}
}
