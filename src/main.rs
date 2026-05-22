#![allow(unused_imports)]

use std::{
	borrow::Cow,
	cell::{Cell, LazyCell},
	env,
	ffi::OsStr,
	fs,
	io::{self, BufRead, Read, Write},
	mem,
	os::unix::prelude::*,
	path::{Path, PathBuf},
	process,
};

#[test]
fn parse_test() {
	// Consecutive spaces are collapsed unless quoted.
	assert_eq!(
		parse_line(r#"hello    world"#).as_slice(),
		["hello", "world"]
	);

	// Spaces are preserved within quotes.
	assert_eq!(
		parse_line(r#"'hello    world'"#).as_slice(),
		["hello    world"]
	);

	// Adjacent quoted strings 'hello' and 'world' are concatenated.
	assert_eq!(parse_line(r#"'hello''world'"#).as_slice(), ["helloworld"]);

	// Empty quotes '' are ignored.
	assert_eq!(parse_line(r#"hello''world"#).as_slice(), ["helloworld"]);

	// Spaces are preserved within double-quotes.
	assert_eq!(
		parse_line(r#""hello    world""#).as_slice(),
		["hello    world"]
	);

	// Quoted strings next to each other are concatenated.
	assert_eq!(parse_line(r#""hello""world""#).as_slice(), ["helloworld"]);

	// Quoted and unquoted strings next to each other are concatenated.
	assert_eq!(parse_line(r#""hello"world"#).as_slice(), ["helloworld"]);

	// Separate arguments.
	assert_eq!(
		parse_line(r#""hello" "world""#).as_slice(),
		["hello", "world"]
	);

	// Single quotes inside are literal.
	assert_eq!(parse_line(r#""shell's test""#).as_slice(), ["shell's test"]);

	// Each \ creates a literal space as part of one argument.
	assert_eq!(
		parse_line(r#"three\ \ \ spaces"#).as_slice(),
		["three   spaces"]
	);

	// The backslash preserves the first space literally, but the shell collapses the subsequent unescaped spaces.
	assert_eq!(
		parse_line(r#"before\     after"#).as_slice(),
		["before ", "after"]
	);
	assert_eq!(parse_line(r#"test\nexample"#).as_slice(), ["testnexample"]); // \n becomes just n.
	assert_eq!(parse_line(r#"hello\\world"#).as_slice(), ["hello\\world"]); // The first backslash escapes the second, and the result is a single literal backslash in the argument.
	assert_eq!(parse_line(r#"\'hello\'"#).as_slice(), ["'hello'"]); // \' makes the single quotes literal characters.

	// Backslashes have no special escaping behavior inside single quotes.
	// Every character (including backslashes) within single quotes is treated literally.
	assert_eq!(
		parse_line(r#"'multiple\\slashes'"#).as_slice(),
		["multiple\\\\slashes"]
	);
	assert_eq!(
		parse_line(r#"'every\"thing_is\"literal'"#).as_slice(),
		["every\\\"thing_is\\\"literal"]
	);

	// Within double quotes, a backslash only escapes certain special characters:
	//", \, $, `, and newline.
	assert_eq!(
		parse_line(r#""A \" inside double quotes""#).as_slice(),
		["A \" inside double quotes"]
	);
	assert_eq!(
		parse_line(r#""A \\ escapes itself""#).as_slice(),
		["A \\ escapes itself"]
	);
	assert_eq!(
		parse_line(r#""A \$ inside double quotes""#).as_slice(),
		["A $ inside double quotes"]
	);
	assert_eq!(
		parse_line(r#""A \` inside double quotes""#).as_slice(),
		["A ` inside double quotes"]
	);
	// TODO: test newline escape in double quotes

	// For all other characters, the backslash is treated literally.
	assert_eq!(
		parse_line(r#""A \ is treated \l\i\t\e\r\a\l\l\y""#).as_slice(),
		[r#"A \ is treated \l\i\t\e\r\a\l\l\y"#]
	);
}

struct Command {
	buf: Vec<String>,
	stdout: Option<PathBuf>,
	stderr: Option<PathBuf>,
}

impl Command {
	pub const fn as_slice(&self) -> &[String] {
		self.buf.as_slice()
	}
}

fn parse_line(line: &str) -> Command {
	let mut src = line.trim();
	let mut buf = vec![];
	let mut wrd = String::new();

	let mut meta = None;

	let mut stdout = None;
	let mut stderr = None;

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
				Some("1>") => stdout = Some(wrd.into()),
				Some("2>") => stderr = Some(wrd.into()),
				Some(_) => unreachable!(),
				None => buf.push(wrd),
			}

			if src.is_empty() {
				break;
			}
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

	Command { buf, stdout, stderr }
}

fn main() -> io::Result<()> {
	let (mut i, mut o, mut e) = (io::stdin().lock(), io::stdout().lock(), io::stderr().lock());
	let mut cmdbuf = String::new();

	loop {
		let mut cmd = {
			write!(o, "$ ")?;
			o.flush()?;
			cmdbuf.clear();
			_ = i.read_line(&mut cmdbuf)?;
			parse_line(cmdbuf.as_str())
		};

		let working_dir = env::current_dir().expect("error getting the current working directory");

		let mut stdout_f = (cmd.stdout.take())
			.map(|path| fs::File::create(path))
			.transpose()
			.unwrap();
		let mut stderr_f = (cmd.stderr.take())
			.map(|path| fs::File::create(path))
			.transpose()
			.unwrap();

		let mut o: &mut dyn Write = stdout_f.as_mut().map_or(&mut o, |f| f);
		let mut e: &mut dyn Write = stderr_f.as_mut().map_or(&mut e, |f| f);

		match &cmd
			.as_slice()
			.iter()
			.map(|s| s.as_ref())
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

				let _exit_status = process::Command::new(program)
					.args(args)
					.stdout(stdout)
					.stderr(stderr)
					.status()?;
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
