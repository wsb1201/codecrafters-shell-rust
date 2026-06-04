#![allow(unused_imports)]

use std::{
	borrow::Cow,
	cell::{Cell, LazyCell},
	env,
	ffi::OsStr,
	fmt, fs,
	io::{self, BufRead, IsTerminal, Read, Write},
	iter, mem,
	os::unix::prelude::*,
	path::{Path, PathBuf},
	process,
};

struct InputChars<R: io::Read> {
	src: io::Bytes<R>,
	utf8buf: [u8; 4],
	utf8len: u8,
}

impl<R: io::Read> InputChars<R> {
	fn new(src: R) -> Self {
		Self {
			#[expect(clippy::unbuffered_bytes)]
			src: src.bytes(),
			utf8buf: [0; _],
			utf8len: 0,
		}
	}
}

impl<R: io::Read> Iterator for InputChars<R> {
	type Item = char;
	fn next(&mut self) -> Option<Self::Item> {
		for b in self.src.by_ref().flatten() {
			self.utf8buf[self.utf8len as usize] = b;
			self.utf8len += 1;
			let chks = self.utf8buf[..self.utf8len as usize].utf8_chunks();
			let Some(ch) = chks.flat_map(|chk| chk.valid().chars()).next() else {
				self.utf8len &= 0b11;
				continue;
			};
			self.utf8len = 0;
			return Some(ch);
		}
		None
	}
}

struct Input<R: io::Read> {
	src: iter::Peekable<InputChars<R>>,
}

enum InputData {
	Char(char),
	CsiSeq(String),
}

impl<R: io::Read> Input<R> {
	fn new(src: R) -> Self {
		Self {
			src: InputChars::new(src).peekable(),
		}
	}

	pub fn next_if(&mut self, f: impl FnOnce(char) -> bool) -> Option<char> {
		self.src.next_if(|&ch| f(ch))
	}
}

impl<R: io::Read> Iterator for Input<R> {
	type Item = InputData;
	fn next(&mut self) -> Option<Self::Item> {
		match self.src.next()? {
			// Introduce an ANSI escape sequence.
			'\x1B' if self.src.next_if(|&ch| ch == '[').is_some() => {
				let mut csibuf = String::new();
				for ch in self.src.by_ref() {
					csibuf.push(ch);
					if ('\x40'..='\x7E').contains(&ch) {
						return Some(InputData::CsiSeq(csibuf));
					}
				}
				None
			}

			ch => Some(InputData::Char(ch)),
		}
	}
}

struct Buffer {
	buf: String,
	idx: usize,
	insert_mode: bool,
}

impl fmt::Display for Buffer {
	#[inline]
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		<String as fmt::Display>::fmt(&self.buf, f)
	}
}

impl io::Seek for Buffer {
	fn seek(&mut self, _pos: io::SeekFrom) -> io::Result<u64> {
		todo!()
	}
}

impl Buffer {
	const fn new() -> Self {
		Self {
			buf: String::new(),
			idx: 0,
			insert_mode: false,
		}
	}

	fn insert(&mut self, ch: char) -> bool {
		let mut replaced = false;
		if self.insert_mode {
			replaced = self.delete();
		}
		self.buf.insert(self.idx, ch);
		self.idx += ch.len_utf8();
		!replaced
	}

	fn delete(&mut self) -> bool {
		let doit = !self.is_cursor_at_end();
		if doit {
			self.buf.remove(self.idx);
		}
		doit
	}

	fn is_cursor_at_end(&self) -> bool {
		self.idx == self.len()
	}

	fn is_cursor_at_start(&self) -> bool {
		self.idx == 0
	}

	fn cursor_to_start(&mut self) -> bool {
		let doit = !self.is_cursor_at_start();
		if doit {
			self.idx = 0
		}
		doit
	}

	fn cursor_to_end(&mut self) -> bool {
		let doit = !self.is_cursor_at_end();
		if doit {
			self.idx = self.len()
		}
		doit
	}

	fn cursor_incr(&mut self) -> bool {
		let doit = !self.is_cursor_at_end();
		if doit {
			self.idx = self.buf.ceil_char_boundary(self.idx + 1);
		}
		doit
	}

	fn cursor_decr(&mut self) -> bool {
		let doit = !self.is_cursor_at_start();
		if doit {
			self.idx = self.buf.floor_char_boundary(self.idx - 1);
		}
		doit
	}

	fn insert_str(&mut self, s: &str) {
		self.buf.insert_str(self.idx, s);
		self.idx += s.len();
	}

	fn get_cursor_prefix_word(&self) -> &str {
		const META_CHARS: &[char] = &[' ', '\t', '\n', '(', ')', '<', '>', '|', '&', ';'];

		let mut s = &self.buf[..self.idx];
		let mut e = s;

		loop {
			if e.starts_with(META_CHARS) {
				e = e.trim_start_matches(META_CHARS);
				s = e;
			}

			if e.is_empty() {
				return s;
			}

			if let Some(sub) = e.strip_prefix('\'') {
				let Some((_, rem)) = sub.split_once('\'') else {
					return s;
				};
				e = rem;
			} else if let Some(sub) = e.strip_prefix('"') {
				e = sub;
				loop {
					let Some(p) = e.find(['"', '\\']) else { return s };
					let (_, b) = e.split_at(p);
					let ch = b.chars().next().unwrap();
					e = b.strip_prefix(ch).unwrap();
					if ch == '"' {
						break;
					}
				}
			} else if let Some(sub) = e.strip_prefix('\\') {
				let Some(ch) = sub.chars().next() else { return s };
				e = sub.strip_prefix(ch).unwrap();
			} else {
				let Some(ch) = e.chars().next() else { return s };
				e = e.strip_prefix(ch).unwrap();
			}
		}
	}

	fn is_cursor_in_first_word(&self) -> bool {
		const META_CHARS: &[char] = &[' ', '\t', '\n', '(', ')', '<', '>', '|', '&', ';'];

		let mut s = self.buf.as_str()[..self.idx].trim_ascii_start();

		if s.ends_with(META_CHARS) {
			return false;
		}

		loop {
			if s.is_empty() {
				return true;
			} else if s.starts_with(META_CHARS) {
				return false;
			}

			if let Some(sub) = s.strip_prefix('\'') {
				let Some((_, rem)) = sub.split_once('\'') else {
					return true;
				};
				s = rem;
			} else if let Some(sub) = s.strip_prefix('"') {
				s = sub;
				loop {
					let Some(p) = s.find(['"', '\\']) else { return true };
					let (_, b) = s.split_at(p);
					let ch = b.chars().next().unwrap();
					s = b.strip_prefix(ch).unwrap();
					if ch == '"' {
						break;
					}
				}
			} else if let Some(sub) = s.strip_prefix('\\') {
				let Some(ch) = sub.chars().next() else { return true };
				s = sub.strip_prefix(ch).unwrap();
			} else {
				let Some(ch) = s.chars().next() else { return true };
				s = s.strip_prefix(ch).unwrap();
			}
		}
	}

	fn len(&self) -> usize {
		self.buf.len()
	}

	fn clear(&mut self) {
		self.buf.clear();
		self.idx = 0;
	}

	fn as_str(&self) -> &str {
		self.buf.as_str()
	}
}

struct Terminal<R: io::Read, W: io::Write> {
	input: Input<R>,
	out: W,
	buf: Buffer,
}

impl<R: io::Read, W: io::Write> Terminal<R, W> {
	fn new(i: R, o: W) -> Self {
		Self {
			input: Input::new(i),
			out: o,
			buf: Buffer::new(),
		}
	}

	fn refresh(&mut self) -> io::Result<()> {
		write!(
			self.out,
			concat!(
				"\x1B7",  // Save cursor position.
				"\x1B[K", // Erase from cursor to end of line.
				"{}",     // Write updated text
				"\x1B8"   // Restore saved cursor position.
			),
			&self.buf.as_str()[self.buf.idx..]
		)?;
		self.flush()
	}

	fn flush(&mut self) -> io::Result<()> {
		self.out.flush()
	}

	fn alert(&mut self) -> io::Result<()> {
		write!(self.out, "\x07")?;
		self.flush()
	}

	fn cursor_right(&mut self) -> io::Result<()> {
		if self.buf.cursor_incr() {
			write!(self.out, "\x1B[C")?;
			self.flush()
		} else {
			self.alert()
		}
	}

	fn cursor_left(&mut self) -> io::Result<()> {
		if self.buf.cursor_decr() {
			write!(self.out, "\x1B[D")?;
			self.flush()
		} else {
			self.alert()
		}
	}

	fn cursor_home(&mut self) -> io::Result<()> {
		if self.buf.cursor_to_start() {
			write!(self.out, "\x1B[3G")?;
			self.flush()
		} else {
			self.alert()
		}
	}

	fn cursor_end(&mut self) -> io::Result<()> {
		if self.buf.cursor_to_end() {
			let col = 3 + self.buf.as_str().chars().count();
			write!(self.out, "\x1B[{col}G")?;
			self.flush()
		} else {
			self.alert()
		}
	}

	fn backspace(&mut self) -> io::Result<()> {
		if self.buf.cursor_decr() && self.buf.delete() {
			write!(self.out, "\x08")?;
			self.refresh()
		} else {
			self.alert()
		}
	}
}

pub(crate) fn prompt(completions: &crate::Completions) -> io::Result<String> {
	let mut t = Terminal::new(io::stdin().lock(), io::stdout().lock());

	write!(t.out, "$ ")?;
	_ = t.flush();

	loop {
		match (t.input.next()).ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))? {
			InputData::CsiSeq(seq) => match seq.as_str() {
				"A" => _ = t.alert(),
				"B" => _ = t.alert(),
				"C" => t.cursor_right()?,
				"D" => t.cursor_left()?,
				"H" => t.cursor_home()?,
				"F" => t.cursor_end()?,
				"2~" => t.buf.insert_mode = !t.buf.insert_mode,
				"3~" => {
					if t.buf.delete() {
						t.refresh()?;
					} else {
						_ = t.alert();
					}
				}
				"5~" => _ = t.alert(),
				"6~" => _ = t.alert(),
				_ => (),
			},

			InputData::Char('\n') => {
				writeln!(t.out)?;
				let ret = t.buf.to_string();
				t.buf.clear();
				return Ok(ret);
			}

			InputData::Char('\t') => {
				let prefix = t.buf.get_cursor_prefix_word();
				let root = if t.buf.is_cursor_in_first_word() {
					&completions.commands
				} else {
					&completions.files
				};

				let Some(min) = root.complete_minimal(prefix) else {
					write!(t.out, "\x07")?;
					_ = t.flush();
					continue;
				};

				if let Some(s) = min.value() {
					let extra = s.strip_prefix(prefix).unwrap();
					t.buf.insert_str(extra);
					write!(t.out, "{extra}")?;

					if !t.buf.is_cursor_at_end() {
						t.refresh()?;
					} else if min.is_leaf() {
						t.buf.insert(' ');
						write!(t.out, " ")?;
					}

					if min.is_leaf() {
						_ = t.flush();
						continue;
					}
				}

				t.alert()?;

				while t.input.next_if(|ch| ch == '\t').is_some() {
					let comp = min.collect_values();
					debug_assert!(comp.len() > 1);

					write!(t.out, "\x1B7")?;
					{
						writeln!(t.out)?;

						let width = 2 + comp.iter().map(|&s| s.len()).max().unwrap();
						let mut sum = 0;
						for i in comp {
							// TODO: dynamic terminal line width
							if sum + width >= 80 {
								writeln!(t.out)?;
								sum = 0;
							}
							write!(t.out, "{i:width$}")?;
							sum += width;
						}
						writeln!(t.out)?;
					}
					write!(t.out, "\x1B[K")?;
					write!(t.out, "$ {}", t.buf)?;
					write!(t.out, "\x1B8")?;
					_ = t.flush();
				}
			}

			InputData::Char('\r') => t.cursor_home()?,

			InputData::Char('\x04') => {
				// May place terminals on standby.
				write!(t.out, "\x04")?;
			}

			InputData::Char('\x08' | '\x7F') => t.backspace()?,

			InputData::Char('\x0B') => {
				// Move down to the next vertical tab stop.
			}

			InputData::Char('\x0C') => {
				// Move down to the top of the next page.
			}

			InputData::Char(..'\x20') => {
				// Unhandled ASCII C0. Does nothing.
			}

			InputData::Char(ch @ '\x20'..) => {
				write!(t.out, "{ch}")?;

				if t.buf.insert(ch) && !t.buf.is_cursor_at_end() {
					t.refresh()?;
				} else {
					_ = t.flush();
				}
			}
		}
	}
}
