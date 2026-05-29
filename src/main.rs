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

#[test]
fn parse_test() {
	let parse_line = |s| {
		parse_line(s)
			.into_iter()
			.filter_map(|f| match f {
				Fragment::Word(s) => Some(s),
				_ => None,
			})
			.collect::<Vec<_>>()
	};

	// Consecutive spaces are collapsed unless quoted.
	assert_eq!(parse_line(r#"hello    world"#), ["hello", "world"]);

	// Spaces are preserved within quotes.
	assert_eq!(parse_line(r#"'hello    world'"#), ["hello    world"]);

	// Adjacent quoted strings 'hello' and 'world' are concatenated.
	assert_eq!(parse_line(r#"'hello''world'"#), ["helloworld"]);

	// Empty quotes '' are ignored.
	assert_eq!(parse_line(r#"hello''world"#), ["helloworld"]);

	// Spaces are preserved within double-quotes.
	assert_eq!(parse_line(r#""hello    world""#), ["hello    world"]);

	// Quoted strings next to each other are concatenated.
	assert_eq!(parse_line(r#""hello""world""#), ["helloworld"]);

	// Quoted and unquoted strings next to each other are concatenated.
	assert_eq!(parse_line(r#""hello"world"#), ["helloworld"]);

	// Separate arguments.
	assert_eq!(parse_line(r#""hello" "world""#), ["hello", "world"]);

	// Single quotes inside are literal.
	assert_eq!(parse_line(r#""shell's test""#), ["shell's test"]);

	// Each \ creates a literal space as part of one argument.
	assert_eq!(parse_line(r#"three\ \ \ spaces"#), ["three   spaces"]);

	// The backslash preserves the first space literally, but the shell collapses the subsequent unescaped spaces.
	assert_eq!(parse_line(r#"before\     after"#), ["before ", "after"]);
	assert_eq!(parse_line(r#"test\nexample"#), ["testnexample"]); // \n becomes just n.
	assert_eq!(parse_line(r#"hello\\world"#), ["hello\\world"]); // The first backslash escapes the second, and the result is a single literal backslash in the argument.
	assert_eq!(parse_line(r#"\'hello\'"#), ["'hello'"]); // \' makes the single quotes literal characters.

	// Backslashes have no special escaping behavior inside single quotes.
	// Every character (including backslashes) within single quotes is treated literally.
	assert_eq!(
		parse_line(r#"'multiple\\slashes'"#),
		["multiple\\\\slashes"]
	);
	assert_eq!(
		parse_line(r#"'every\"thing_is\"literal'"#),
		["every\\\"thing_is\\\"literal"]
	);

	// Within double quotes, a backslash only escapes certain special characters:
	//", \, $, `, and newline.
	assert_eq!(
		parse_line(r#""A \" inside double quotes""#),
		["A \" inside double quotes"]
	);
	assert_eq!(
		parse_line(r#""A \\ escapes itself""#),
		["A \\ escapes itself"]
	);
	assert_eq!(
		parse_line(r#""A \$ inside double quotes""#),
		["A $ inside double quotes"]
	);
	assert_eq!(
		parse_line(r#""A \` inside double quotes""#),
		["A ` inside double quotes"]
	);
	// TODO: test newline escape in double quotes

	// For all other characters, the backslash is treated literally.
	assert_eq!(
		parse_line(r#""A \ is treated \l\i\t\e\r\a\l\l\y""#),
		[r#"A \ is treated \l\i\t\e\r\a\l\l\y"#]
	);
}

enum Fragment {
	Word(String),
	RedirectFd {
		fd: RawFd,
		path: PathBuf,
		truncate: bool,
	},
}

fn parse_line(line: &str) -> Vec<Fragment> {
	let mut src = line.trim();
	let mut buf = vec![];
	let mut wrd = String::new();

	let mut meta = None;

	/// A character that, when unquoted, separates words.
	const META_CHARS: &[char] = &[' ', '\t', '\n', '(', ')', '<', '>', '|', '&', ';'];

	'l: loop {
		if src.is_empty() || src.starts_with([' ', '\t']) {
			src = src.strip_prefix([' ', '\t']).unwrap_or(src);
			if wrd.is_empty() {
				if src.is_empty() {
					break;
				}
				continue;
			}

			let wrd = mem::take(&mut wrd);

			match meta.take() {
				Some("1>") => buf.push(Fragment::RedirectFd {
					fd: 1,
					path: wrd.into(),
					truncate: true,
				}),
				Some("2>") => buf.push(Fragment::RedirectFd {
					fd: 2,
					path: wrd.into(),
					truncate: true,
				}),
				Some("1>>") => buf.push(Fragment::RedirectFd {
					fd: 1,
					path: wrd.into(),
					truncate: false,
				}),
				Some("2>>") => buf.push(Fragment::RedirectFd {
					fd: 2,
					path: wrd.into(),
					truncate: false,
				}),
				Some(_) => unreachable!(),
				None => buf.push(Fragment::Word(wrd)),
			}

			if src.is_empty() {
				break;
			}
		} else if let Some(sub) = src.strip_prefix("1>>").or_else(|| src.strip_prefix(">>")) {
			src = sub;
			meta = Some("1>>")
		} else if let Some(sub) = src.strip_prefix("2>>") {
			src = sub;
			meta = Some("2>>")
		} else if let Some(sub) = src.strip_prefix("1>").or_else(|| src.strip_prefix('>')) {
			src = sub;
			meta = Some("1>")
		} else if let Some(sub) = src.strip_prefix("2>") {
			src = sub;
			meta = Some("2>")
		} else if let Some(sub) = src.strip_prefix('\'') {
			let (quoted, rem) = sub.split_once('\'').unwrap_or((src, ""));
			wrd.push_str(quoted);
			src = rem;
		} else if let Some(sub) = src.strip_prefix('"') {
			src = sub;
			loop {
				let Some(p) = src.find(['"', '\\']) else {
					wrd.push_str(src);
					continue 'l;
				};

				let (a, b) = src.split_at(p);
				wrd.push_str(a);

				let ch = b.chars().next().unwrap();
				src = b.strip_prefix(ch).unwrap();
				if ch == '"' {
					break;
				}

				let Some(ch) = src.chars().next() else { continue 'l };
				src = src.strip_prefix(ch).unwrap();
				if !matches!(ch, '"' | '\\' | '$' | '`' | '\n') {
					wrd.push('\\');
				}
				wrd.push(ch);
			}
		} else if let Some(sub) = src.strip_prefix('\\') {
			let Some(ch) = sub.chars().next() else { continue 'l };
			src = sub.strip_prefix(ch).unwrap();
			wrd.push(ch);
		} else {
			let Some(ch) = src.chars().next() else { continue 'l };
			wrd.push(ch);
			src = src.strip_prefix(ch).unwrap();
		}
	}

	buf
}

fn create_redirection_target(path: impl AsRef<Path>, truncate: bool) -> io::Result<fs::File> {
	fs::OpenOptions::new()
		.create(true)
		.write(true)
		.truncate(truncate)
		.append(!truncate)
		.open(path)
}

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
		let mut cmd = {
			let cmdbuf = interactive::next()?;
			parse_line(cmdbuf.as_str())
		};

		let working_dir = env::current_dir().expect("error getting the current working directory");

		let mut stdout_f = cmd
			.iter()
			.filter_map(|f| match f {
				Fragment::RedirectFd { fd: 1, path, truncate } => {
					Some(create_redirection_target(path, *truncate))
				}
				_ => None,
			})
			.last()
			.transpose()
			.unwrap();

		let mut stderr_f = cmd
			.iter()
			.filter_map(|f| match f {
				Fragment::RedirectFd { fd: 2, path, truncate } => {
					Some(create_redirection_target(path, *truncate))
				}
				_ => None,
			})
			.last()
			.transpose()
			.unwrap();

		let mut o: &mut dyn Write = stdout_f.as_mut().map_or(&mut o, |f| f);
		let mut e: &mut dyn Write = stderr_f.as_mut().map_or(&mut e, |f| f);

		match &cmd
			.iter()
			.filter_map(|f| match f {
				Fragment::Word(s) => Some(s.as_str()),
				_ => None,
			})
			.collect::<Vec<_>>()[..]
		{
			[] => continue,

			// builtin command `exit`
			["exit", ..] => break Ok(()),

			// builtin command `echo`
			["echo", args @ ..] => {
				let echo: String = args.join(" ");
				writeln!(o, "{echo}")?;
			}

			// builtin command `type`
			["type", args @ ..] => builtin_cmd_type(&mut o, &mut e, args)?,

			// builtin command `pwd`
			["pwd", ..] => writeln!(o, "{}", working_dir.display())?,

			// builtin command `cd`
			["cd", args @ ..] => builtin_cmd_cd(&mut o, &mut e, args)?,

			// external program?
			[program, args @ ..] if find_executable(program).is_some() => {
				let stdout = stdout_f
					.map(process::Stdio::from)
					.unwrap_or(process::Stdio::inherit());
				let stderr = stderr_f
					.map(process::Stdio::from)
					.unwrap_or(process::Stdio::inherit());

				let mut extcmd = process::Command::new(program);
				extcmd.args(args).stdout(stdout).stderr(stderr);
				if let Some(termios_mode) = &termios_mode {
					_ = termios_mode.restore_orig();
				}
				let _exit_status = extcmd.status()?;
				if let Some(termios_mode) = &termios_mode {
					_ = termios_mode.set_repl_mode();
				}
			}

			// unavailable command
			[cmd, ..] => writeln!(&mut o, "{cmd}: command not found")?,
		}
	}
}

fn env_path() -> Vec<Box<Path>> {
	let Some(var) = env::var_os("PATH") else {
		return vec![];
	};
	var.as_encoded_bytes()
		.split(|&b| matches!(b, b':'))
		.map(|bytes| {
			// SAFETY:
			// `bytes` originates from `OsStr::as_encoded_bytes`
			// and is split at the non-empty UTF-8 substring ':'.
			Box::from(Path::new(unsafe {
				OsStr::from_encoded_bytes_unchecked(bytes)
			}))
		})
		.collect()
}

fn read_dirs<P: AsRef<Path>>(path: impl Iterator<Item = P>) -> impl Iterator<Item = fs::DirEntry> {
	path.filter(|p| p.as_ref().is_dir())
		.flat_map(fs::read_dir)
		.flat_map(IntoIterator::into_iter)
		.flatten()
}

fn is_executable_file(path: impl AsRef<Path>) -> bool {
	path.as_ref().metadata().is_ok_and(|metadata| {
		let permissions = metadata.permissions();
		metadata.is_file() && permissions.mode() & 0o111 != 0
	})
}

fn find_executable(program: &str) -> Option<PathBuf> {
	let path = LazyCell::new(env_path);
	read_dirs(path.iter())
		.filter_map(|file| (program == file.file_name()).then(|| file.path()))
		.find(|path| is_executable_file(path))
}

fn builtin_cmd_cd(o: &mut dyn Write, e: &mut dyn Write, args: &[&str]) -> io::Result<()> {
	let Some(arg) = args.first() else {
		if !args.is_empty() {
			writeln!(o, "cd: Too many arguments")?;
		}
		return Ok(());
	};

	let path = Path::new(arg);
	let path: Cow<Path> = (path.strip_prefix("~").ok())
		.and_then(|rel| {
			let mut home = env::var_os("HOME").map(PathBuf::from)?;
			home.push(rel);
			Some(home.into())
		})
		.unwrap_or(path.into());
	let path = path.as_ref();
	let p = path.display();

	let Err(err) = env::set_current_dir(path) else {
		return Ok(());
	};

	match err.kind() {
		io::ErrorKind::NotFound => {
			writeln!(e, "cd: {p}: No such file or directory")
		}
		_ => {
			writeln!(o, "cd: {p}: {err}")
		}
	}
}

fn builtin_cmd_type(o: &mut dyn Write, e: &mut dyn Write, args: &[&str]) -> io::Result<()> {
	match args {
		[] => Ok(()),

		[builtin @ ("exit" | "echo" | "type" | "pwd" | "cd"), ..] => {
			writeln!(o, "{builtin} is a shell builtin")
		}

		// The command is not a builtin, look for it in PATH.
		[cmd, ..] if let Some(exec) = find_executable(cmd) => {
			// The file exists and has execute permissions.
			writeln!(o, "{} is {}", cmd, exec.display())
		}

		// No executable was found in PATH.
		[unknown, ..] => writeln!(e, "{unknown}: not found"),
	}
}
