//! Desktop Entry `Exec` values for sessions: the Desktop Entry
//! Specification's quoting and field codes, and the command handed to greetd.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
/// What field codes may refer to.
pub(crate) struct Entry<'a> {
    pub name: &'a str,
    pub icon: Option<&'a str>,
    pub path: &'a str,
}

// Decode Desktop Entry string escapes before applying Exec quoting rules.
fn unescape_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

enum Word {
    /// An unquoted word, still carrying field codes.
    Plain(String),
    /// A quoted word: literal, field codes are not allowed inside it.
    Quoted(String),
}

fn split(exec: &str) -> Result<Vec<Word>, String> {
    let mut words = Vec::new();
    let mut chars = exec.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| *c == ' ' || *c == '\t') {
            chars.next();
        }
        let Some(&first) = chars.peek() else { break };
        if first == '"' {
            chars.next();
            let mut word = String::new();
            loop {
                match chars.next() {
                    None => {
                        return Err(tr("shell-its-command-has-an-unterminated-quote").into());
                    }
                    Some('"') => break,
                    Some('\\') => match chars.next() {
                        Some(c @ ('"' | '`' | '$' | '\\')) => word.push(c),
                        _ => {
                            return Err(tr(
                                "shell-its-command-has-an-invalid-escape-inside-quotes",
                            )
                            .into());
                        }
                    },
                    Some(c) => word.push(c),
                }
            }
            let mut literal = String::with_capacity(word.len());
            let mut rest = word.chars();
            while let Some(c) = rest.next() {
                if c == '%' && rest.next() != Some('%') {
                    return Err(tr("shell-its-command-uses-a-field-code-inside-quotes").into());
                }
                literal.push(c);
            }
            if chars.peek().is_some_and(|c| *c != ' ' && *c != '\t') {
                return Err(tr("shell-its-command-has-text-directly-after-a-closing-quote").into());
            }
            words.push(Word::Quoted(literal));
        } else {
            let mut word = String::new();
            while let Some(&c) = chars.peek() {
                if c == ' ' || c == '\t' {
                    break;
                }
                if c == '"' {
                    return Err(tr("shell-its-command-has-a-quote-inside-a-word").into());
                }
                word.push(c);
                chars.next();
            }
            words.push(Word::Plain(word));
        }
    }
    Ok(words)
}

fn expand(word: &str, entry: &Entry<'_>) -> Result<Vec<String>, String> {
    // A code standing alone: file and URL lists expand to nothing (a session
    // is started without files), %i to two arguments or none.
    match word {
        "%f" | "%F" | "%u" | "%U" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => {
            return Ok(Vec::new());
        }
        "%i" => {
            return Ok(entry
                .icon
                .map(|icon| vec!["--icon".to_owned(), icon.to_owned()])
                .unwrap_or_default());
        }
        _ => {}
    }
    let mut out = String::new();
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some('c') => out.push_str(entry.name),
            Some('k') => out.push_str(entry.path),
            Some('f' | 'F' | 'u' | 'U' | 'd' | 'D' | 'n' | 'N' | 'v' | 'm') => {}
            Some('i') => {
                return Err(tr("shell-its-command-uses-i-inside-a-word").into());
            }
            Some(other) => {
                return Err(
                    tr!("shell-its-command-uses-the-unknown-field-code", "arg0" => format!("{:}", other)),
                );
            }
            None => return Err(tr("shell-its-command-ends-with-a-lone").into()),
        }
    }
    Ok(vec![out])
}

// The argument vector an `Exec` value describes.
pub(crate) fn argv(exec: &str, entry: &Entry<'_>) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    for word in split(&unescape_value(exec))? {
        match word {
            Word::Quoted(text) => args.push(text),
            Word::Plain(text) => args.extend(expand(&text, entry)?),
        }
    }
    if args.first().is_none_or(std::string::String::is_empty) {
        return Err(tr("shell-its-command-names-no-program").into());
    }
    Ok(args)
}

fn shell_safe(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+=:,@%".contains(c))
}

// Quotes arguments for greetd's profile-sourced shell command; plain words remain directly
// executable.
pub(crate) fn greetd_command(args: &[String]) -> Vec<String> {
    args.iter()
        .map(|word| {
            if shell_safe(word) {
                word.clone()
            } else {
                format!("'{}'", word.replace('\'', "'\\''"))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY: Entry<'static> = Entry {
        name: "My Desktop",
        icon: Some("desktop"),
        path: "/usr/share/wayland-sessions/my.desktop",
    };

    fn parse(exec: &str) -> Result<Vec<String>, String> {
        argv(exec, &ENTRY)
    }

    #[test]
    fn exec_behaviour() {
        // Quoting keeps spaces and escapes.
        {
            assert_eq!(
                parse(r#""/opt/My Session/bin/start" --title "A \"quoted\" name" --cost "\$5""#)
                    .unwrap(),
                vec![
                    "/opt/My Session/bin/start",
                    "--title",
                    "A \"quoted\" name",
                    "--cost",
                    "$5"
                ]
            );
            // `\s` is a string escape undone before Exec splitting, so it only
            // keeps a space inside quotes.
            assert_eq!(parse(r"run\sthis --x").unwrap(), vec!["run", "this", "--x"]);
            assert_eq!(
                parse(r#""run\sthis" --x"#).unwrap(),
                vec!["run this", "--x"]
            );
            assert_eq!(
                parse("  enzui-session   --wayland  ").unwrap(),
                vec!["enzui-session", "--wayland"]
            );
        }
        // Field codes expand or vanish.
        {
            assert_eq!(
                parse("env A=1 run %U 100%%").unwrap(),
                vec!["env", "A=1", "run", "100%"]
            );
            assert_eq!(
                parse("start %f %F %u %d %D %n %N %v %m --x").unwrap(),
                vec!["start", "--x"]
            );
            assert_eq!(
                parse("start %i --name=%c --from=%k").unwrap(),
                vec![
                    "start",
                    "--icon",
                    "desktop",
                    "--name=My Desktop",
                    "--from=/usr/share/wayland-sessions/my.desktop"
                ]
            );
            assert_eq!(
                argv(
                    "start %i",
                    &Entry {
                        icon: None,
                        ..ENTRY
                    }
                )
                .unwrap(),
                vec!["start"]
            );
            assert_eq!(parse(r#"start "100%%""#).unwrap(), vec!["start", "100%"]);
        }
        // Malformed commands are refused with a reason.
        {
            for bad in [
                r#"start "unterminated"#,
                "start %z",
                "start 50%",
                r#"start "%f""#,
                r#"start a"b""#,
                r#""a"b"#,
                "%f",
                "",
                r#"start "\q""#,
            ] {
                assert!(parse(bad).is_err(), "{bad:?} was accepted");
            }
            assert_eq!(
                parse("start %z")
                    .unwrap_err()
                    .replace(['\u{2068}', '\u{2069}'], ""),
                "its command uses the unknown field code %z"
            );
        }
        // Greetd gets shell safe words.
        {
            let args = parse(r#""/opt/My Session/start" --name "it's" plain --x=1"#).unwrap();
            assert_eq!(
                greetd_command(&args),
                vec![
                    "'/opt/My Session/start'",
                    "--name",
                    r"'it'\''s'",
                    "plain",
                    "--x=1"
                ]
            );
            let joined = greetd_command(&args).join(" ");
            let out = std::process::Command::new("/bin/sh")
                .args([
                    "-c",
                    &format!("for a in {joined}; do printf '<%s>' \"$a\"; done"),
                ])
                .output()
                .unwrap();
            assert_eq!(
                String::from_utf8(out.stdout).unwrap(),
                "</opt/My Session/start><--name><it's><plain><--x=1>"
            );
        }
    }
}
