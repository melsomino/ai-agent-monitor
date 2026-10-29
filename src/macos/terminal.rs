//! Route terminal sessions to their existing host, never to an editor.
use crate::{agents::sessions::Session, app::process::output_with_timeout};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

const SCHEME: &str = "agent-monitor-terminal";

pub fn target(session: &Session) -> String {
    let mut url = url::Url::parse("agent-monitor-terminal://session").unwrap();
    url.query_pairs_mut()
        .append_pair("agent", session.agent)
        .append_pair("id", &session.id)
        .append_pair("cwd", &session.cwd.to_string_lossy());
    url.to_string()
}

#[derive(Debug)]
struct Process {
    pid: u32,
    parent: u32,
    tty: String,
    command: String,
    args: String,
}

fn process_lines(text: &str) -> HashMap<u32, Process> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent = fields.next()?.parse().ok()?;
            let tty = fields.next()?.to_owned();
            let command = fields.collect::<Vec<_>>().join(" ");
            Some((
                pid,
                Process {
                    pid,
                    parent,
                    tty,
                    command,
                    args: String::new(),
                },
            ))
        })
        .collect()
}

fn choose<'a>(
    processes: &'a HashMap<u32, Process>,
    agent: &str,
    id: &str,
    directories: &HashMap<u32, PathBuf>,
    cwd: &Path,
) -> Option<&'a Process> {
    let candidates: Vec<_> = processes
        .values()
        .filter(|p| {
            p.tty != "??"
                && Path::new(&p.command)
                    .file_name()
                    .is_some_and(|name| name == agent)
        })
        .collect();
    let exact: Vec<_> = candidates
        .iter()
        .copied()
        .filter(|p| p.args.split_whitespace().any(|arg| arg == id))
        .collect();
    if exact.len() == 1 {
        return Some(exact[0]);
    }
    let matches: Vec<_> = candidates
        .into_iter()
        .filter(|p| {
            // A resumed client already bound to a different thread must never
            // be selected just because it shares this project's directory.
            !p.args.split_whitespace().any(|arg| {
                arg.len() == 36
                    && arg.bytes().filter(|byte| *byte == b'-').count() == 4
                    && arg != id
            }) && !cwd.as_os_str().is_empty()
                && directories
                    .get(&p.pid)
                    .is_some_and(|directory| directory == cwd)
        })
        .collect();
    (matches.len() == 1).then(|| matches[0])
}

fn host(processes: &HashMap<u32, Process>, mut pid: u32) -> Option<PathBuf> {
    for _ in 0..32 {
        let process = processes.get(&pid)?;
        if let Some(end) = process.command.find(".app/") {
            return Some(PathBuf::from(&process.command[..end + 4]));
        }
        if process.parent == pid {
            return None;
        }
        pid = process.parent;
    }
    None
}

fn directories(text: &str) -> HashMap<u32, PathBuf> {
    let mut result = HashMap::new();
    let mut pid = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix('p') {
            pid = value.parse::<u32>().ok();
        } else if let (Some(pid), Some(path)) = (pid, line.strip_prefix('n')) {
            result.insert(pid, PathBuf::from(path));
        }
    }
    result
}

pub fn open(value: &str) -> Option<bool> {
    let url = url::Url::parse(value).ok()?;
    if url.scheme() != SCHEME {
        return None;
    }
    let query: HashMap<_, _> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let agent = query.get("agent")?.clone();
    if !matches!(agent.as_str(), "codex" | "claude") {
        return Some(false);
    }
    let id = query.get("id")?.clone();
    let cwd = PathBuf::from(query.get("cwd")?);
    // Process discovery and Apple Events must not stall the menu's main thread.
    std::thread::spawn(move || {
        let _ = focus(&agent, &id, &cwd);
    });
    Some(true)
}

fn focus(agent: &str, id: &str, cwd: &Path) -> anyhow::Result<()> {
    let timeout = Duration::from_secs(3);
    let (_, text, _) = output_with_timeout(
        Command::new("/bin/ps").args(["-axo", "pid=,ppid=,tty=,comm="]),
        timeout,
    )?;
    let mut processes = process_lines(&text);
    let (_, args, _) = output_with_timeout(
        Command::new("/bin/ps").args(["-axo", "pid=,args="]),
        timeout,
    )?;
    for line in args.lines() {
        let line = line.trim_start();
        if let Some((pid, args)) = line.split_once(char::is_whitespace)
            && let Ok(pid) = pid.parse::<u32>()
            && let Some(process) = processes.get_mut(&pid)
        {
            process.args = args.trim_start().to_owned();
        }
    }
    // lsof limits cwd discovery to the agent's processes and the current user.
    // SAFETY: geteuid has no preconditions; discovery stays scoped to this user.
    let uid = unsafe { libc::geteuid() }.to_string();
    let (_, text, _) = output_with_timeout(
        Command::new("/usr/sbin/lsof").args([
            "-nP", "-a", "-u", &uid, "-c", agent, "-d", "cwd", "-F", "pn",
        ]),
        timeout,
    )?;
    let directories = directories(&text);
    let initial = if agent == "codex" {
        crate::agents::codex::initial_directory(id)
    } else {
        None
    };
    let process = choose(
        &processes,
        agent,
        id,
        &directories,
        initial.as_deref().unwrap_or(cwd),
    );
    let Some(process) = process else {
        let mut apps: Vec<_> = processes
            .values()
            .filter(|process| {
                process.tty != "??"
                    && Path::new(&process.command)
                        .file_name()
                        .is_some_and(|name| name == agent)
            })
            .filter_map(|process| host(&processes, process.pid))
            .collect();
        apps.sort();
        apps.dedup();
        if apps.len() == 1 {
            output_with_timeout(
                Command::new("/usr/bin/open").arg("-a").arg(&apps[0]),
                timeout,
            )?;
        }
        return Ok(());
    };
    let Some(app) = host(&processes, process.pid) else {
        return Ok(());
    };
    let tty = format!("/dev/{}", process.tty);
    if app.file_name().is_some_and(|name| name == "Terminal.app") {
        output_with_timeout(
            Command::new("/usr/bin/osascript").args(["-e", TERMINAL_SCRIPT, "--", &tty]),
            Duration::from_secs(10),
        )?;
    } else if app.file_name().is_some_and(|name| name == "iTerm.app") {
        output_with_timeout(
            Command::new("/usr/bin/osascript").args(["-e", ITERM_SCRIPT, "--", &tty]),
            Duration::from_secs(10),
        )?;
    } else {
        // Hosts without a tab scripting interface still receive application focus.
        output_with_timeout(Command::new("/usr/bin/open").arg("-a").arg(app), timeout)?;
    }
    Ok(())
}

const TERMINAL_SCRIPT: &str = r#"on run argv
    set wantedTTY to item 1 of argv
    tell application id "com.apple.Terminal"
        repeat with w in windows
            repeat with t in tabs of w
                if tty of t is wantedTTY then
                    set selected tab of w to t
                    set index of w to 1
                    activate
                    return
                end if
            end repeat
        end repeat
        activate
    end tell
end run"#;
const ITERM_SCRIPT: &str = r#"on run argv
    set wantedTTY to item 1 of argv
    tell application id "com.googlecode.iterm2"
        repeat with w in windows
            repeat with t in tabs of w
                repeat with s in sessions of t
                    if tty of s is wantedTTY then
                        select s
                        select t
                        select w
                        activate
                        return
                    end if
                end repeat
            end repeat
        end repeat
        activate
    end tell
end run"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resume_identity_wins_over_directory_and_ambiguous_directories_are_not_guessed() {
        let mut processes = process_lines(
            "10 2 ttys001 codex\n11 3 ttys002 /bin/codex\n12 1 ?? /bin/codex\n2 1 ?? /System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal\n3 1 ?? /Applications/Warp.app/Contents/MacOS/stable",
        );
        processes.get_mut(&11).unwrap().args = "codex".into();
        processes.get_mut(&10).unwrap().args =
            "codex resume 01a188b1-a902-7540-9b67-94747ca1ccf2".into();
        let dirs = HashMap::from([(10, PathBuf::from("/a")), (11, PathBuf::from("/a"))]);
        assert_eq!(
            choose(
                &processes,
                "codex",
                "01a188b1-a902-7540-9b67-94747ca1ccf2",
                &dirs,
                Path::new("/b")
            )
            .unwrap()
            .tty,
            "ttys001"
        );
        assert_eq!(
            choose(&processes, "codex", "other", &dirs, Path::new("/a"))
                .unwrap()
                .pid,
            11
        );
        processes.get_mut(&10).unwrap().args = "codex".into();
        assert!(choose(&processes, "codex", "other", &dirs, Path::new("/a")).is_none());
        assert!(choose(&processes, "codex", "other", &dirs, Path::new("")).is_none());
        assert_eq!(
            host(&processes, 10).unwrap(),
            Path::new("/System/Applications/Utilities/Terminal.app")
        );
        assert_eq!(
            host(&processes, 11).unwrap(),
            Path::new("/Applications/Warp.app")
        );
    }
    #[test]
    fn cwd_records_are_bound_to_their_process() {
        assert_eq!(
            directories("p10\nfcwd\nn/work/one\np20\nfcwd\nn/work/two with spaces\n"),
            HashMap::from([
                (10, PathBuf::from("/work/one")),
                (20, PathBuf::from("/work/two with spaces"))
            ])
        );
    }
}
