//! Agent command specification.

const MAX_DISPLAY_ARG_CHARS: usize = 64;
const REDACTED_ARGUMENT: &str = "<redacted>";

/// Concrete command to execute for an underlying agent CLI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    program: &'static str,
    args: Vec<String>,
    stdin: Option<String>,
}

impl CommandSpec {
    /// Creates a command specification.
    #[must_use]
    pub fn new(program: &'static str, args: Vec<String>) -> Self {
        Self {
            program,
            args,
            stdin: None,
        }
    }

    /// Creates a command specification that sends prompt text through stdin.
    #[must_use]
    pub fn with_stdin(program: &'static str, args: Vec<String>, stdin: String) -> Self {
        Self {
            program,
            args,
            stdin: Some(stdin),
        }
    }

    /// Returns the executable program.
    #[must_use]
    pub const fn program(&self) -> &'static str {
        self.program
    }

    /// Returns the executable arguments.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Returns stdin payload for commands that use stdin prompt transport.
    #[must_use]
    pub fn stdin(&self) -> Option<&str> {
        self.stdin.as_deref()
    }

    /// Returns the command as user-facing text.
    #[must_use]
    pub fn display(&self) -> String {
        let mut parts = std::iter::once(self.program.to_owned())
            .chain(self.args.iter().map(|arg| display_arg(arg)))
            .collect::<Vec<_>>();
        if self.stdin.is_some() {
            parts.push("<stdin>".to_owned());
        }
        parts.join(" ")
    }
}

fn display_arg(arg: &str) -> String {
    if arg.chars().any(char::is_control) || arg.chars().count() > MAX_DISPLAY_ARG_CHARS {
        REDACTED_ARGUMENT.to_owned()
    } else {
        arg.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::CommandSpec;

    #[test]
    fn exposes_command_fields() {
        let command = CommandSpec::new("agent", vec!["arg".to_owned()]);

        assert_eq!(command.program(), "agent");
        assert_eq!(command.args(), ["arg"]);
        assert_eq!(command.stdin(), None);
        assert_eq!(command.display(), "agent arg");

        let prompt = CommandSpec::new("agent", vec!["exec".to_owned(), "line\nprompt".to_owned()]);
        assert_eq!(prompt.display(), "agent exec <redacted>");
        let stdin = CommandSpec::with_stdin("agent", vec!["exec".to_owned()], "prompt".to_owned());
        assert_eq!(stdin.stdin(), Some("prompt"));
        assert_eq!(stdin.display(), "agent exec <stdin>");
    }
}
