//! Makes the bridge start by itself when the computer does — the one thing a person has to do once
//! so that "the PC is on" is enough (Go: internal/service). It only writes a per-user service file
//! and runs the system's own commands for it (systemd user service, launchd agent, a Windows
//! scheduled task): no root, nothing outside the user's account. Everything it would do can be
//! shown first with --dry-run.

use std::io::Write;
use std::path::{Path, PathBuf};

/// What the service files are made from.
#[derive(Debug, Clone)]
pub struct Env {
    /// "linux", "macos" or "windows" (`std::env::consts::OS`).
    pub os: String,
    pub home: PathBuf,
    pub user: String,
    pub uid: u32,
    /// The bridge's own path: the service runs this very file.
    pub exe: PathBuf,
    /// Where the `claude` command lives (None if not found). A service starts with a minimal PATH,
    /// so that directory has to be named in the service itself.
    pub claude_dir: Option<PathBuf>,
}

/// What install / uninstall / status do on one system.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// The service file to write (None when the system keeps no such file).
    pub file: Option<PathBuf>,
    pub content: String,
    /// Commands, as argument lists, for each action.
    pub install: Vec<Vec<String>>,
    pub uninstall: Vec<Vec<String>>,
    pub status: Vec<Vec<String>>,
    /// For the person, printed after an install.
    pub notes: Vec<String>,
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn cmd(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

/// The directories the bridge's `claude` must be found in, without repeats.
fn path_list(env: &Env, sep: &str, dirs: &[String]) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    for d in env.claude_dir.iter().map(|d| s(d)).chain(dirs.iter().cloned()) {
        if !d.is_empty() && seen.insert(d.clone()) {
            out.push(d);
        }
    }
    out.join(sep)
}

/// The plan for `env.os`.
pub fn plan_for(env: &Env) -> Result<Plan, String> {
    if env.exe.as_os_str().is_empty() || !env.exe.is_absolute() {
        return Err(format!("the bridge's path {:?} is not absolute", s(&env.exe)));
    }
    let bad = |t: &str| t.contains(['\n', '\r', '"']);
    if bad(&s(&env.exe)) || env.claude_dir.as_ref().is_some_and(|d| bad(&s(d))) {
        return Err("a path with a line break or a quote cannot go into a service file".into());
    }
    match env.os.as_str() {
        "linux" => Ok(linux(env)),
        "macos" => Ok(macos(env)),
        "windows" => Ok(windows(env)),
        other => Err(format!("no service support for {other}: run `spellink-bridge serve` from your own startup mechanism")),
    }
}

fn linux(env: &Env) -> Plan {
    let path = path_list(env, ":", &[s(&env.home.join(".local/bin")), "/usr/local/bin".into(), "/usr/bin".into(), "/bin".into()]);
    let unit = format!(
        "# Written by `spellink-bridge service install`; safe to delete with `service uninstall`.
[Unit]
Description=Spellink bridge (Supernote and an AI)
After=network-online.target
Wants=network-online.target

[Service]
ExecStart={} serve
Restart=on-failure
RestartSec=5s
Environment=PATH={path}

[Install]
WantedBy=default.target
",
        s(&env.exe)
    );
    Plan {
        file: Some(env.home.join(".config/systemd/user/spellink-bridge.service")),
        content: unit,
        install: vec![
            cmd(&["systemctl", "--user", "daemon-reload"]),
            cmd(&["systemctl", "--user", "enable", "--now", "spellink-bridge"]),
            // without this, a user service stops when the user logs out and starts only at login
            cmd(&["loginctl", "enable-linger", &env.user]),
        ],
        uninstall: vec![cmd(&["systemctl", "--user", "disable", "--now", "spellink-bridge"]), cmd(&["systemctl", "--user", "daemon-reload"])],
        status: vec![cmd(&["systemctl", "--user", "--no-pager", "status", "spellink-bridge"])],
        notes: vec!["Log: journalctl --user -u spellink-bridge".into(), "Check the connection: spellink-bridge check".into()],
    }
}

fn xml_escape(t: &str) -> String {
    t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn macos(env: &Env) -> Plan {
    let path = path_list(env, ":", &[s(&env.home.join(".local/bin")), "/opt/homebrew/bin".into(), "/usr/local/bin".into(), "/usr/bin".into(), "/bin".into()]);
    let log_file = s(&env.home.join("Library/Logs/spellink-bridge.log"));
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- Written by spellink-bridge service install; remove with service uninstall. -->
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>app.spellink.bridge</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>serve</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key><string>{}</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>{}</string>
  <key>StandardErrorPath</key><string>{}</string>
</dict>
</plist>
"#,
        xml_escape(&s(&env.exe)),
        xml_escape(&path),
        xml_escape(&log_file),
        xml_escape(&log_file)
    );
    let file = env.home.join("Library/LaunchAgents/app.spellink.bridge.plist");
    let domain = format!("gui/{}", env.uid);
    let label = format!("{domain}/app.spellink.bridge");
    Plan {
        file: Some(file.clone()),
        content: plist,
        install: vec![
            cmd(&["launchctl", "bootout", &label]), // ignore "not loaded": a re-install replaces it
            cmd(&["launchctl", "bootstrap", &domain, &s(&file)]),
        ],
        uninstall: vec![cmd(&["launchctl", "bootout", &label])],
        status: vec![cmd(&["launchctl", "print", &label])],
        notes: vec![format!("Log: {log_file}"), "Check the connection: spellink-bridge check".into()],
    }
}

fn windows(env: &Env) -> Plan {
    const TASK: &str = "Spellink bridge";
    let tr = format!("\"{}\" serve", s(&env.exe));
    Plan {
        file: None,
        content: String::new(),
        install: vec![cmd(&["schtasks", "/Create", "/F", "/SC", "ONLOGON", "/RL", "LIMITED", "/TN", TASK, "/TR", &tr]), cmd(&["schtasks", "/Run", "/TN", TASK])],
        uninstall: vec![cmd(&["schtasks", "/End", "/TN", TASK]), cmd(&["schtasks", "/Delete", "/F", "/TN", TASK])],
        status: vec![cmd(&["schtasks", "/Query", "/TN", TASK, "/V", "/FO", "LIST"])],
        notes: vec!["The bridge starts when you log on, and runs while you are logged on.".into(), "Windows is untested: if something does not work, run `spellink-bridge serve` in a terminal and tell us.".into()],
    }
}

/// Runs one command: its output and whether it worked; tests replace it.
pub type Runner<'a> = &'a mut dyn FnMut(&[String]) -> (Vec<u8>, Result<(), String>);

/// Runs the command for real.
pub fn exec_runner(args: &[String]) -> (Vec<u8>, Result<(), String>) {
    match std::process::Command::new(&args[0]).args(&args[1..]).output() {
        Ok(o) => {
            let mut out = o.stdout;
            out.extend_from_slice(&o.stderr);
            let r = if o.status.success() { Ok(()) } else { Err(format!("{}", o.status)) };
            (out, r)
        }
        Err(e) => (vec![], Err(e.to_string())),
    }
}

/// Carries out one action of the plan, writing what it does to `w`. With `dry` it only shows.
/// `write` stores the service file; `remove` deletes it.
pub fn do_action(action: &str, p: &Plan, dry: bool, run: Runner, write: &mut dyn FnMut(&Path, &str) -> Result<(), String>, remove: &mut dyn FnMut(&Path) -> Result<(), String>, w: &mut dyn Write) -> Result<(), String> {
    let cmds = match action {
        "install" => &p.install,
        "uninstall" => &p.uninstall,
        "status" => &p.status,
        other => return Err(format!("unknown action {other:?} (install, uninstall, status)")),
    };
    let io = |e: std::io::Error| e.to_string();
    if action == "install" {
        if let Some(file) = &p.file {
            writeln!(w, "write {}", file.display()).map_err(io)?;
            if dry {
                write!(w, "----\n{}----\n", p.content).map_err(io)?;
            } else {
                write(file, &p.content).map_err(|e| format!("writing {}: {e}", file.display()))?;
            }
        }
    }
    for c in cmds {
        writeln!(w, "run   {}", c.join(" ")).map_err(io)?;
        if dry {
            continue;
        }
        let (out, res) = run(c);
        let text = String::from_utf8_lossy(&out).trim().to_string();
        if !text.is_empty() && action == "status" {
            writeln!(w, "{text}").map_err(io)?;
        }
        // Taking down something that is not there (launchd bootout on a first install) is fine.
        if let Err(e) = res {
            if !(action == "install" && c.len() > 1 && c[1] == "bootout") {
                return Err(format!("{}: {e}: {text}", c.join(" ")));
            }
        }
    }
    if action == "uninstall" {
        if let Some(file) = &p.file {
            writeln!(w, "remove {}", file.display()).map_err(io)?;
            if !dry {
                remove(file)?;
            }
        }
    }
    if action == "install" {
        for n in &p.notes {
            writeln!(w, "{n}").map_err(io)?;
        }
    }
    Ok(())
}

/// `spellink-bridge service install|uninstall|status [--dry-run]`.
pub fn command(args: &[String]) -> Result<(), String> {
    let dry = args.iter().any(|a| a == "--dry-run" || a == "-n");
    let rest: Vec<&String> = args.iter().filter(|a| *a != "--dry-run" && *a != "-n").collect();
    if rest.len() != 1 {
        return Err("usage: spellink-bridge service install|uninstall|status [--dry-run]".into());
    }
    let exe = std::env::current_exe().and_then(std::fs::canonicalize).map_err(|e| e.to_string())?;
    // A binary from `cargo run` lives in a build directory and would be gone after a clean.
    if exe.starts_with(std::env::temp_dir()) || exe.components().any(|c| c.as_os_str() == "target") {
        return Err(format!("{} is a temporary build: install the program first (for example to ~/.local/bin) and run the service command from there", exe.display()));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).ok_or("no home directory")?;
    let env = Env {
        os: std::env::consts::OS.into(),
        home,
        user: std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default(),
        uid: std::process::Command::new("id").arg("-u").output().ok().and_then(|o| String::from_utf8(o.stdout).ok()).and_then(|t| t.trim().parse().ok()).unwrap_or(0),
        exe,
        claude_dir: crate::app::look_path("claude").and_then(|p| p.parent().map(Path::to_path_buf)),
    };
    let plan = plan_for(&env)?;
    let mut write = |path: &Path, content: &str| -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, content).map_err(|e| e.to_string())
    };
    let mut remove = |path: &Path| -> Result<(), String> {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    };
    do_action(rest[0], &plan, dry, &mut exec_runner, &mut write, &mut remove, &mut std::io::stdout())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(os: &str) -> Env {
        Env { os: os.into(), home: "/home/pat".into(), user: "pat".into(), uid: 1000, exe: "/home/pat/.local/bin/spellink-bridge".into(), claude_dir: Some("/opt/claude/bin".into()) }
    }

    #[test]
    fn the_linux_unit_runs_the_bridge_by_its_full_path_with_claude_on_the_path() {
        let p = plan_for(&env("linux")).unwrap();
        for want in ["ExecStart=/home/pat/.local/bin/spellink-bridge serve", "Restart=on-failure", "Environment=PATH=/opt/claude/bin:/home/pat/.local/bin:/usr/local/bin:/usr/bin:/bin", "WantedBy=default.target"] {
            assert!(p.content.contains(want), "the unit lacks {want:?}:\n{}", p.content);
        }
        assert_eq!(p.file.as_deref(), Some(Path::new("/home/pat/.config/systemd/user/spellink-bridge.service")), "a per-user file, not a system one");
        let joined: String = p.install.iter().chain(&p.uninstall).map(|c| c.join(" ") + "\n").collect();
        assert!(!joined.contains("sudo") && !joined.contains("/etc/"), "nothing outside the user's account, no root: {joined}");
        assert!(joined.contains("loginctl enable-linger pat"), "without linger it would stop when the user logs out: {joined}");
    }

    #[test]
    fn the_path_has_no_repeats_and_survives_no_claude() {
        let mut e = env("linux");
        e.claude_dir = Some("/home/pat/.local/bin".into()); // already one of the usual directories
        let p = plan_for(&e).unwrap();
        assert_eq!(p.content.matches("/home/pat/.local/bin:").count(), 1, "a directory appears once: {}", p.content);
        e.claude_dir = None;
        assert!(plan_for(&e).unwrap().content.contains("Environment=PATH=/home/pat/.local/bin:"), "without claude found, the usual directories");
    }

    #[test]
    fn the_mac_agent_is_a_valid_plist_with_escaped_values() {
        let mut e = env("macos");
        e.home = "/Users/pat".into();
        e.exe = "/Users/pat/bin & tools/spellink-bridge".into();
        let p = plan_for(&e).unwrap();
        assert!(p.content.contains("<string>/Users/pat/bin &amp; tools/spellink-bridge</string>"), "XML special characters must be escaped:\n{}", p.content);
        for want in ["app.spellink.bridge", "<key>RunAtLoad</key><true/>", "<key>KeepAlive</key><true/>"] {
            assert!(p.content.contains(want), "the agent lacks {want:?}");
        }
        assert_eq!(p.file.as_deref(), Some(Path::new("/Users/pat/Library/LaunchAgents/app.spellink.bridge.plist")));
        assert_eq!(p.install[1].join(" "), "launchctl bootstrap gui/1000 /Users/pat/Library/LaunchAgents/app.spellink.bridge.plist");
    }

    #[test]
    fn windows_uses_a_scheduled_task_and_quotes_the_path() {
        let mut e = env("windows");
        e.exe = "/Program Files/Spellink/spellink-bridge.exe".into();
        let p = plan_for(&e).unwrap();
        assert!(p.file.is_none(), "a scheduled task keeps no file of ours");
        let got = p.install[0].join("\0");
        assert!(got.contains("/TR\0\"/Program Files/Spellink/spellink-bridge.exe\" serve") && got.contains("ONLOGON") && got.contains("LIMITED"), "limited rights, at logon, the path quoted: {:?}", p.install[0]);
    }

    #[test]
    fn paths_that_cannot_be_written_safely_are_refused() {
        let cases: Vec<(&str, Box<dyn Fn(&mut Env)>)> = vec![
            ("a relative path", Box::new(|e| e.exe = "spellink-bridge".into())),
            ("an empty path", Box::new(|e| e.exe = "".into())),
            ("a line break in the path", Box::new(|e| e.exe = "/a\nExecStart=/bin/evil".into())),
            ("a quote in the path", Box::new(|e| e.exe = "/a\"b".into())),
            ("a line break in claude's", Box::new(|e| e.claude_dir = Some("/x\n[Service]".into()))),
        ];
        for (name, mutate) in cases {
            let mut e = env("linux");
            mutate(&mut e);
            assert!(plan_for(&e).is_err(), "{name} must be refused");
        }
        assert!(plan_for(&env("plan9")).unwrap_err().contains("serve"), "an unsupported system says what to do instead");
    }

    #[derive(Default)]
    struct Recorder {
        ran: Vec<String>,
        fail_on: Option<&'static str>,
        written: Vec<(PathBuf, String)>,
        removed: Vec<PathBuf>,
    }

    impl Recorder {
        fn run(&mut self, args: &[String]) -> (Vec<u8>, Result<(), String>) {
            let c = args.join(" ");
            self.ran.push(c.clone());
            if self.fail_on.is_some_and(|f| c.contains(f)) {
                return (b"it did not work".to_vec(), Err("exit status 1".into()));
            }
            (b"active (running)".to_vec(), Ok(()))
        }
    }

    fn act(action: &str, p: &Plan, dry: bool, r: &mut Recorder) -> (Result<(), String>, String) {
        let (written, removed) = (std::cell::RefCell::new(vec![]), std::cell::RefCell::new(vec![]));
        let ran = std::cell::RefCell::new(std::mem::take(&mut r.ran));
        let (fail_on, mut out) = (r.fail_on, Vec::new());
        let result = {
            let mut run = |a: &[String]| {
                let mut rec = Recorder { fail_on, ..Recorder::default() };
                let res = rec.run(a);
                ran.borrow_mut().extend(rec.ran);
                res
            };
            let mut write = |p: &Path, c: &str| -> Result<(), String> {
                written.borrow_mut().push((p.to_path_buf(), c.to_string()));
                Ok(())
            };
            let mut remove = |p: &Path| -> Result<(), String> {
                removed.borrow_mut().push(p.to_path_buf());
                Ok(())
            };
            do_action(action, p, dry, &mut run, &mut write, &mut remove, &mut out)
        };
        r.ran = ran.into_inner();
        r.written.extend(written.into_inner());
        r.removed.extend(removed.into_inner());
        (result, String::from_utf8(out).unwrap())
    }

    #[test]
    fn install_writes_the_file_then_runs_the_commands_in_order() {
        let p = plan_for(&env("linux")).unwrap();
        let mut r = Recorder::default();
        let (res, out) = act("install", &p, false, &mut r);
        res.unwrap();
        assert_eq!(r.written, vec![(p.file.clone().unwrap(), p.content.clone())], "the unit must be written");
        assert!(r.ran.len() == 3 && r.ran[0].starts_with("systemctl --user daemon-reload") && r.ran[2].starts_with("loginctl enable-linger"), "commands: {:?}", r.ran);
        assert!(out.contains("spellink-bridge check"), "the person is told how to check it worked: {out}");
    }

    #[test]
    fn a_dry_run_changes_nothing_and_shows_everything() {
        let p = plan_for(&env("linux")).unwrap();
        let mut r = Recorder::default();
        let mut all = String::new();
        for action in ["install", "uninstall", "status"] {
            let (res, out) = act(action, &p, true, &mut r);
            res.unwrap();
            all += &out;
        }
        assert!(r.ran.is_empty() && r.written.is_empty() && r.removed.is_empty(), "a dry run must do nothing: ran {:?}", r.ran);
        let file = p.file.as_ref().unwrap().display().to_string();
        for want in [format!("write {file}"), "ExecStart=".into(), "run   systemctl --user enable --now spellink-bridge".into(), format!("remove {file}")] {
            assert!(all.contains(&want), "a dry run shows {want:?}");
        }
    }

    #[test]
    fn a_failing_command_stops_the_install_and_names_itself() {
        let p = plan_for(&env("linux")).unwrap();
        let mut r = Recorder { fail_on: Some("enable --now"), ..Recorder::default() };
        let (res, _) = act("install", &p, false, &mut r);
        let e = res.unwrap_err();
        assert!(e.contains("enable --now") && e.contains("it did not work"), "the error says which command and why: {e}");
        assert_eq!(r.ran.len(), 2, "nothing runs after the one that failed: {:?}", r.ran);
    }

    #[test]
    fn a_first_install_on_a_mac_ignores_that_there_was_nothing_to_take_down() {
        let p = plan_for(&env("macos")).unwrap();
        let mut r = Recorder { fail_on: Some("bootout"), ..Recorder::default() };
        act("install", &p, false, &mut r).0.expect("a first install has nothing loaded to boot out");
        assert!(r.ran.len() == 2 && r.ran[1].contains("bootstrap"), "it goes on to load the agent: {:?}", r.ran);
    }

    #[test]
    fn uninstall_takes_it_down_and_removes_the_file() {
        let p = plan_for(&env("linux")).unwrap();
        let mut r = Recorder::default();
        act("uninstall", &p, false, &mut r).0.unwrap();
        assert_eq!(r.removed, vec![p.file.clone().unwrap()]);
        assert!(r.ran[0].starts_with("systemctl --user disable --now"), "stop it before deleting its file: {:?}", r.ran);
    }

    #[test]
    fn status_shows_the_systems_answer_and_an_unknown_action_is_refused() {
        let p = plan_for(&env("linux")).unwrap();
        let mut r = Recorder::default();
        let (res, out) = act("status", &p, false, &mut r);
        assert!(res.is_ok() && out.contains("active (running)"), "{res:?} {out:?}");
        assert!(act("restart", &p, false, &mut Recorder::default()).0.is_err(), "an unknown action must be refused");
    }
}
