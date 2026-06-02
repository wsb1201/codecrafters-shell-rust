use std::{
	fmt,
	io::{self, Read, Write},
};

struct Input<R: io::Read> {
	src: io::Bytes<R>,
	utf8buf: [u8; 4],
	utf8len: u8,
}

impl<R: io::Read> Input<R> {
	fn new(src: R) -> Self {
		Self {
			src: src.bytes(),
			utf8buf: [0; _],
			utf8len: 0,
		}
	}
}

impl<R: io::Read> Iterator for Input<R> {
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

	fn backspace_delete(&mut self) -> bool {
		self.cursor_decr() && self.delete()
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

	const fn len(&self) -> usize {
		self.buf.len()
	}

	fn clear(&mut self) {
		self.buf.clear();
		self.idx = 0;
	}

	const fn as_str(&self) -> &str {
		self.buf.as_str()
	}
}

pub fn next(completions: &crate::trie::Trie) -> io::Result<String> {
	let mut input = Input::new(io::stdin().lock()).peekable();
	let mut o = io::stdout().lock();

	let mut buf = Buffer::new();
	let mut csi: Option<String> = None;

	loop {
		write!(o, "$ ")?;
		_ = o.flush();

		let mut refresh = false;
		loop {
			if refresh {
				refresh = false;
				write!(o, "\x1B7")?;
				write!(o, "\x1B[K")?;
				write!(o, "{}", &buf.as_str()[buf.idx..])?;
				write!(o, "\x1B8")?;
				_ = o.flush();
			}

			let Some(ch) = input.next() else {
				return Err(io::ErrorKind::UnexpectedEof.into());
			};

			if let Some(cmd) = &mut csi {
				cmd.push(ch);

				if ('\x40'..='\x7E').contains(&ch) {
					match ch {
						'A' => {
							// UP
							write!(o, "\x07")?;
							_ = o.flush();
						}
						'B' => {
							// DOWN
							write!(o, "\x07")?;
							_ = o.flush();
						}
						'C' => {
							// RIGHT
							if buf.cursor_incr() {
								write!(o, "\x1B[{cmd}")?;
							} else {
								write!(o, "\x07")?;
							}
							_ = o.flush();
						}
						'D' => {
							// LEFT
							if buf.cursor_decr() {
								write!(o, "\x1B[{cmd}")?;
							} else {
								write!(o, "\x07")?;
							}
							_ = o.flush();
						}
						'H' => {
							// HOME
							if buf.cursor_to_start() {
								write!(o, "\x1B[3G")?;
							} else {
								write!(o, "\x07")?;
							}
							_ = o.flush();
						}
						'F' => {
							// END
							if buf.cursor_to_end() {
								let col = 3 + buf.as_str().chars().count();
								write!(o, "\x1B[{col}G")?;
							} else {
								write!(o, "\x07")?;
							}
							_ = o.flush();
						}
						'~' if cmd == "2~" => {
							// INSERT
							buf.insert_mode = !buf.insert_mode;
						}
						'~' if cmd == "3~" => {
							// DELETE
							if buf.delete() {
								refresh = true;
							} else {
								write!(o, "\x07")?;
								_ = o.flush();
							}
						}
						'~' if cmd == "5~" => {
							// PG UP
							write!(o, "\x07")?;
							_ = o.flush();
						}
						'~' if cmd == "6~" => {
							// PG DN
							write!(o, "\x07")?;
							_ = o.flush();
						}
						_ => (),
					}
					csi.take();
				}
				continue;
			}

			if ch < '\x20' {
				// ASCII C0
				match ch as u8 {
					b'\n' => {
						// ENTER
						writeln!(o)?; // Move down to the left column on the next line.
						let ret = buf.to_string();
						buf.clear();
						return Ok(ret);
					}

					b'\t' => {
						let prefix = &buf.as_str()[..buf.idx];
						let Some(min) = completions.complete_minimal(prefix) else {
							write!(o, "\x07")?;
							_ = o.flush();
							continue;
						};

						if let Some(comp) = min.value()
							&& comp != prefix
						{
							let extra = comp
								.strip_prefix(prefix)
								.expect("completion should start with previous content");
							buf.insert_str(extra);
							write!(o, "{extra}")?;
							refresh = true;

							if min.is_leaf() && buf.is_cursor_at_end() {
								buf.insert(' ');
								write!(o, " ")?;
							}
							continue;
						}

						write!(o, "\x07")?;
						_ = o.flush();

						if input.next_if(|&ch| ch == '\t').is_none() {
							continue;
						}

						let comp = min.collect_values();
						debug_assert!(comp.len() > 1);

						write!(o, "\x1B7")?;
						{
							writeln!(o)?;
							let width = 2 + comp.iter().map(|&s| s.len()).max().unwrap();
							let mut sum = 0;
							for i in comp {
								// TODO: dynamic terminal line width
								if sum + width >= 80 {
									writeln!(o)?;
									sum = 0;
								}
								write!(o, "{i:width$}")?;
								sum += width;
							}
							writeln!(o)?;
						}
						write!(o, "\x1B[K")?;
						write!(o, "$ {buf}")?;
						write!(o, "\x1B8")?;
						_ = o.flush();
					}

					b'\x1B' if input.next_if(|&ch| ch == '[').is_some() => {
						csi = Some(String::new());
					}

					b'\r' => (),      // Move to column zero while staying on the same line.
					b'\0' => (),      // Does nothing.
					0x01..0x04 => (), // Does nothing.
					0x04 => (),       // May place terminals on standby.
					0x05..0x08 => (), // Does nothing.
					0x08 => (), // Move one position leftwards. Next character may replace the character that was there.
					0x0B => (), // Move down to the next vertical tab stop.
					0x0C => (), // Move down to the top of the next page.
					0x0E..0x1B => (), // Does nothing.
					0x1B => (), // Introduce an ANSI escape sequence.
					0x1C..0x20 => (), // Does nothing.
					_ => unreachable!(),
				}

				continue;
			}

			if ch == '\x7F' {
				// BACKSPACE
				if buf.backspace_delete() {
					write!(o, "\x08")?;
					refresh = true;
				} else {
					write!(o, "\x07")?;
					_ = o.flush();
				}
				continue;
			}

			write!(o, "{ch}")?;
			_ = o.flush();

			if buf.insert(ch) && buf.idx < buf.len() {
				refresh = true;
			}
		}
	}
}

fn get_caret_notation(ch: u8) -> &'static str {
	match ch {
		b'\0' => "^@",
		0x01 => "^A",
		0x02 => "^B",
		0x03 => "^C",
		0x04 => "^D",
		0x05 => "^E",
		0x06 => "^F",
		0x07 => "^G",
		0x08 => "^H",
		b'\t' => "^I",
		b'\n' => "^J",
		0x0B => "^K",
		0x0C => "^L",
		b'\r' => "^M",
		0x0E => "^N",
		0x0F => "^O",
		0x10 => "^P",
		0x11 => "^Q",
		0x12 => "^R",
		0x13 => "^S",
		0x14 => "^T",
		0x15 => "^U",
		0x16 => "^V",
		0x17 => "^W",
		0x18 => "^X",
		0x19 => "^Y",
		0x1A => "^Z",
		0x1B => "^[",
		0x1C => "^\\",
		0x1D => "^]",
		0x1E => "^^",
		0x1F => "^_",
		_ => "",
	}
}

fn get_control_picture(ch: u8) -> char {
	match ch {
		b'\0' => '␀',
		0x01 => '␁',
		0x02 => '␂',
		0x03 => '␃',
		0x04 => '␄',
		0x05 => '␅',
		0x06 => '␆',
		0x07 => '␇',
		0x08 => '␈',
		b'\t' => '␉',
		b'\n' => '␊',
		0x0B => '␋',
		0x0C => '␌',
		b'\r' => '␍',
		0x0E => '␎',
		0x0F => '␏',
		0x10 => '␐',
		0x11 => '␑',
		0x12 => '␒',
		0x13 => '␓',
		0x14 => '␔',
		0x15 => '␕',
		0x16 => '␖',
		0x17 => '␗',
		0x18 => '␘',
		0x19 => '␙',
		0x1A => '␚',
		0x1B => '␛',
		0x1C => '␜',
		0x1D => '␝',
		0x1E => '␞',
		0x1F => '␟',
		_ => '\0',
	}
}
