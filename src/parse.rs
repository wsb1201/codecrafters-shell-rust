use std::{mem, os::fd::RawFd, path::PathBuf};

pub(crate) enum Fragment {
	Word(String),
	RedirectFd {
		fd: RawFd,
		path: PathBuf,
		truncate: bool,
	},
}

pub(crate) fn parse_line(line: &str) -> Vec<Fragment> {
	let mut src = line.trim();
	let mut buf = vec![];
	let mut wrd = String::new();

	let mut meta = None;

	/// A character that, when unquoted, separates words.
	const _META_CHARS: &[char] = &[' ', '\t', '\n', '(', ')', '<', '>', '|', '&', ';'];

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
