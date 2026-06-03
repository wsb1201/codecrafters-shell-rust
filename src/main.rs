use std::{
	borrow::Cow,
	cell::LazyCell,
	env,
	ffi::OsStr,
	fs,
	io::{self, IsTerminal, Write},
	os::unix::prelude::*,
	path::{Path, PathBuf},
	process,
};

mod interactive;
mod parse;
mod termios;
mod trie;

fn main() -> io::Result<()> {
	let (i, mut o, mut e) = (io::stdin(), io::stdout(), io::stderr());
	let termios_mode = i
		.is_terminal()
		.then(|| TermiosMode::new(i.as_fd()))
		.transpose()?;

	let mut completions = trie::Trie::new();
	completions.insert("exit".into());
	completions.insert("echo".into());
	completions.insert("type".into());
	completions.insert("pwd".into());
	completions.insert("cd".into());

	for program in read_dirs(env_path().into_iter())
		.filter_map(|file| is_executable_file(file.path()).then(|| file.file_name()))
	{
		if let Ok(program) = program.into_string() {
			completions.insert(program);
		}
	}

	loop {
		let cmd = interactive::prompt(&completions)?;
		let cmd = parse::parse_line(cmd.as_str());

		let working_dir = env::current_dir().expect("error getting the current working directory");

		let mut stdout_f = cmd
			.iter()
			.filter_map(|f| match f {
				parse::Fragment::RedirectFd { fd: 1, path, truncate } => {
					Some(create_redirection_target(path, *truncate))
				}
				_ => None,
			})
			.next_back()
			.transpose()
			.unwrap();

		let mut stderr_f = cmd
			.iter()
			.filter_map(|f| match f {
				parse::Fragment::RedirectFd { fd: 2, path, truncate } => {
					Some(create_redirection_target(path, *truncate))
				}
				_ => None,
			})
			.next_back()
			.transpose()
			.unwrap();

		let mut o: &mut dyn Write = stdout_f.as_mut().map_or(&mut o, |f| f);
		let mut e: &mut dyn Write = stderr_f.as_mut().map_or(&mut e, |f| f);

		match &cmd
			.iter()
			.filter_map(|f| match f {
				parse::Fragment::Word(s) => Some(s.as_str()),
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

struct TermiosMode<'a> {
	fd: BorrowedFd<'a>,
	termios_orig: libc::termios,
	termios_repl: libc::termios,
}

impl<'a> TermiosMode<'a> {
	fn new(fd: BorrowedFd<'a>) -> io::Result<Self> {
		let termios_orig = termios::tcgetattr(fd)?;

		let mut t = termios_orig;
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

fn create_redirection_target(path: impl AsRef<Path>, truncate: bool) -> io::Result<fs::File> {
	fs::OpenOptions::new()
		.create(true)
		.write(true)
		.truncate(truncate)
		.append(!truncate)
		.open(path)
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
