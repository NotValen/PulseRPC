//! Command-line parsing (hand-rolled to avoid a clap-sized binary).

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Run,
    Install,
    Uninstall,
    SetId,
    Validate,
    Rules,
    Probe,
    Help,
    Version,
}

#[derive(Debug, Clone)]
pub struct Args {
    pub command: Command,
    pub config: Option<PathBuf>,
    pub id: Option<String>,
    pub dir: Option<PathBuf>,
    pub quiet: bool,
    pub verbose: bool,
    pub no_watch: bool,
    pub no_autostart: bool,
    pub start: bool,
    pub once: bool,
    pub dry_run: bool,
    pub tray: bool,
    pub headless: bool,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            command: Command::Run,
            config: None,
            id: None,
            dir: None,
            quiet: false,
            verbose: false,
            no_watch: false,
            no_autostart: false,
            start: false,
            once: false,
            dry_run: false,
            tray: false,
            headless: false,
        }
    }
}

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Args, String> {
    let mut out = Args::default();
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        let mut value = |name: &str| -> Result<String, String> {
            iter.next()
                .filter(|v| !v.starts_with('-'))
                .ok_or_else(|| format!("{name} requires a value"))
        };

        match arg.as_str() {
            "--install" => out.command = Command::Install,
            "--uninstall" => out.command = Command::Uninstall,
            "--set-id" => {
                out.command = Command::SetId;
                out.id = Some(value("--set-id")?);
            }
            "--validate" | "--check" => out.command = Command::Validate,
            "--rules" | "--list" => out.command = Command::Rules,
            "--probe" | "--detect" => out.command = Command::Probe,
            "--help" | "-h" | "/?" => out.command = Command::Help,
            "--version" | "-V" => out.command = Command::Version,
            "--once" => out.once = true,
            "--dry-run" => out.dry_run = true,
            "--tray" => out.tray = true,
            "--headless" => out.headless = true,
            "--gui" => out.headless = false,
            "--quiet" | "-q" => out.quiet = true,
            "--verbose" | "-v" => out.verbose = true,
            "--no-watch" => out.no_watch = true,
            "--no-autostart" => out.no_autostart = true,
            "--start" => out.start = true,
            "--config" | "-c" => out.config = Some(PathBuf::from(value("--config")?)),
            "--id" => out.id = Some(value("--id")?),
            "--dir" => out.dir = Some(PathBuf::from(value("--dir")?)),
            other => {
                if let Some(v) = other.strip_prefix("--config=") {
                    out.config = Some(PathBuf::from(v));
                } else if let Some(v) = other.strip_prefix("--set-id=") {
                    out.command = Command::SetId;
                    out.id = Some(v.to_string());
                } else if let Some(v) = other.strip_prefix("--id=") {
                    out.id = Some(v.to_string());
                } else if let Some(v) = other.strip_prefix("--dir=") {
                    out.dir = Some(PathBuf::from(v));
                } else {
                    return Err(format!("unknown argument `{other}`"));
                }
            }
        }
    }

    Ok(out)
}

pub const HELP: &str = "\
PulseRPC — automatic Discord Rich Presence for 90+ desktop apps

USAGE:
    pulserpc [OPTIONS]

OPTIONS:
    -c, --config <DIR>   Config directory (default: next to the exe, else
                         %LOCALAPPDATA%\\PulseRPC\\config)
        --id <ID>        Override the Discord application id for this run
        --set-id <ID>    Save an application id into config.toml and exit
        --tray           Start in the notification area without opening the window
        --headless       Run without the window (console / service style)
        --once           Detect once, print the activity JSON and exit
        --dry-run        Run normally but print activities instead of sending
        --probe          Show every running app and which rule wins
        --rules          List all configured applications and exit
        --validate       Parse the config, print warnings and exit
        --no-watch       Disable hot reload of config files
    -q, --quiet          Errors and warnings only
    -v, --verbose        Debug logging
    -h, --help           Show this help
    -V, --version        Print version

The GUI starts by default: set your Discord Application ID there, then leave
PulseRPC in the tray. Closing the window keeps it running; Quit really exits.

INSTALLER:
        --install        Copy the app to %LOCALAPPDATA%\\Programs\\PulseRPC,
                         install default config, enable autostart and register
                         an uninstaller
        --dir <DIR>       Target directory for --install
        --no-autostart    Do not add the startup entry
        --start          Launch the app after --install
        --uninstall      Remove startup entry, registry entry and app files

QUICK START:
    1. Create a Discord application at https://discord.com/developers/applications
    2. Run `pulserpc` and paste its Application ID into the window
    3. Leave PulseRPC running — the presence follows your active application

Config is hot-reloadable: edit config.toml or apps/*.toml while it runs.
";

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Args, String> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_flags() {
        let a = p(&["--install", "--dir", "C:\\Apps\\AP", "--no-autostart"]).unwrap();
        assert_eq!(a.command, Command::Install);
        assert_eq!(a.dir.unwrap(), PathBuf::from("C:\\Apps\\AP"));
        assert!(a.no_autostart);
    }

    #[test]
    fn parses_equals_form() {
        let a = p(&["--config=E:\\cfg", "--id=123"]).unwrap();
        assert_eq!(a.config.unwrap(), PathBuf::from("E:\\cfg"));
        assert_eq!(a.id.unwrap(), "123");
    }

    #[test]
    fn parses_set_id() {
        let a = p(&["--set-id", "123456789012345678"]).unwrap();
        assert_eq!(a.command, Command::SetId);
        assert_eq!(a.id.as_deref(), Some("123456789012345678"));

        let b = p(&["--set-id=123456789012345678"]).unwrap();
        assert_eq!(b.command, Command::SetId);
        assert_eq!(b.id.as_deref(), Some("123456789012345678"));
    }

    #[test]
    fn rejects_unknown() {
        assert!(p(&["--nope"]).is_err());
    }

    #[test]
    fn missing_value() {
        assert!(p(&["--config"]).is_err());
    }
}
