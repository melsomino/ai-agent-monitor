//! Route terminal sessions to their existing host, never to an editor.
use crate::{agents::sessions::Session, app::process::output_with_timeout};
use std::{
    collections::{HashMap, HashSet},
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
                && directories.contains_key(&p.pid)
                && crate::agents::tracking::is_agent_process(agent, &p.command)
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
    let mut cwd = false;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix('p') {
            pid = value.parse::<u32>().ok();
            cwd = false;
        } else if let Some(fd) = line.strip_prefix('f') {
            cwd = fd == "cwd";
        } else if cwd && let (Some(pid), Some(path)) = (pid, line.strip_prefix('n')) {
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

fn processes() -> anyhow::Result<HashMap<u32, Process>> {
    let timeout = Duration::from_secs(3);
    let (status, text, _) = output_with_timeout(
        Command::new("/bin/ps").args(["-axo", "pid=,ppid=,tty=,comm="]),
        timeout,
    )?;
    anyhow::ensure!(status == Some(0), "Could not inspect terminal clients");
    let mut processes = process_lines(&text);
    let (status, args, _) = output_with_timeout(
        Command::new("/bin/ps").args(["-axo", "pid=,args="]),
        timeout,
    )?;
    anyhow::ensure!(
        status == Some(0),
        "Could not inspect terminal client identities"
    );
    for line in args.lines() {
        let line = line.trim_start();
        if let Some((pid, args)) = line.split_once(char::is_whitespace)
            && let Ok(pid) = pid.parse::<u32>()
            && let Some(process) = processes.get_mut(&pid)
        {
            process.args = args.trim_start().to_owned();
        }
    }
    Ok(processes)
}

/// A daemon-held rollout is not sufficient: every visible terminal session
/// needs a live foreground client. One client can account for only one session.
pub(crate) fn retain_live_codex(sessions: &mut Vec<Session>, files: &str) -> anyhow::Result<()> {
    if !sessions
        .iter()
        .any(|session| session.client == crate::agents::sessions::Client::Terminal)
    {
        return Ok(());
    }
    let processes = processes()?;
    let directories = directories(files);
    let candidates: Vec<_> = sessions
        .iter()
        .filter(|session| session.client == crate::agents::sessions::Client::Terminal)
        .map(|session| {
            (
                session.id.clone(),
                crate::agents::codex::initial_directory(&session.id)
                    .unwrap_or_else(|| session.cwd.clone()),
                session.updated_at,
            )
        })
        .collect();
    let live = live_ids(&processes, &directories, &candidates);
    sessions.retain(|session| {
        session.client != crate::agents::sessions::Client::Terminal || live.contains(&session.id)
    });
    Ok(())
}

fn live_ids(
    processes: &HashMap<u32, Process>,
    directories: &HashMap<u32, PathBuf>,
    sessions: &[(String, PathBuf, i64)],
) -> HashSet<String> {
    let mut clients: Vec<_> = processes
        .values()
        .filter(|process| {
            process.tty != "??"
                && directories.contains_key(&process.pid)
                && Path::new(&process.command)
                    .file_name()
                    .is_some_and(|name| name == "codex")
        })
        .collect();
    clients.sort_by_key(|process| process.pid);
    let mut live = HashSet::new();
    // Explicit resume identities win even when sessions share a directory.
    clients.retain(|process| {
        if let Some((id, _, _)) = sessions
            .iter()
            .find(|(id, _, _)| process.args.split_whitespace().any(|arg| arg == id))
        {
            live.insert(id.clone());
            return false;
        }
        // An explicitly resumed thread outside this snapshot cannot keep a
        // different transcript in the same directory visible.
        !process
            .args
            .split_whitespace()
            .any(|arg| arg.len() == 36 && arg.bytes().filter(|byte| *byte == b'-').count() == 4)
    });
    let mut remaining: Vec<_> = sessions
        .iter()
        .filter(|(id, _, _)| !live.contains(id))
        .collect();
    remaining.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    for (id, cwd, _) in remaining {
        if !cwd.as_os_str().is_empty()
            && let Some(index) = clients
                .iter()
                .position(|process| directories.get(&process.pid) == Some(cwd))
        {
            clients.remove(index);
            live.insert(id.clone());
        }
    }
    live
}

fn focus(agent: &str, id: &str, cwd: &Path) -> anyhow::Result<()> {
    let timeout = Duration::from_secs(3);
    let processes = processes()?;
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
    #[test]
    fn versioned_claude_binary_routes_to_its_terminal() {
        let processes = super::process_lines(
            "10 2 ttys001 /Users/test/.local/share/claude/versions/2.1.3\n2 1 ?? /Applications/iTerm.app/Contents/MacOS/iTerm2",
        );
        let directories =
            std::collections::HashMap::from([(10, std::path::PathBuf::from("/work"))]);
        let process = super::choose(
            &processes,
            "claude",
            "chat",
            &directories,
            std::path::Path::new("/work"),
        )
        .unwrap();
        assert_eq!(process.pid, 10);
        assert_eq!(
            super::host(&processes, process.pid).unwrap(),
            std::path::Path::new("/Applications/iTerm.app")
        );
        assert!(
            super::choose(
                &processes,
                "codex",
                "chat",
                &directories,
                std::path::Path::new("/work")
            )
            .is_none()
        );
    }

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
    fn daemon_rollouts_do_not_keep_closed_terminal_sessions_alive() {
        let mut processes =
            process_lines("10 2 ttys001 codex\n20 1 ?? /bin/codex\n30 3 ttys002 codex");
        let dirs = HashMap::from([
            (10, PathBuf::from("/project")),
            (20, PathBuf::from("/closed")),
            (30, PathBuf::from("/resumed")),
        ]);
        let resumed = "01a10d25-27a3-73a3-838d-9dfb24104803";
        processes.get_mut(&30).unwrap().args = format!("codex resume {resumed}");
        let sessions = vec![
            ("old".into(), PathBuf::from("/project"), 1),
            ("current".into(), PathBuf::from("/project"), 2),
            ("closed".into(), PathBuf::from("/closed"), 3),
            (resumed.into(), PathBuf::from("/resumed"), 1),
            (
                "closed-in-resumed-directory".into(),
                PathBuf::from("/resumed"),
                9,
            ),
        ];
        assert_eq!(
            live_ids(&processes, &dirs, &sessions),
            HashSet::from(["current".into(), resumed.into()])
        );
        processes.remove(&10);
        assert_eq!(
            live_ids(&processes, &dirs, &sessions),
            HashSet::from([resumed.into()])
        );
        processes.remove(&30);
        assert!(live_ids(&processes, &dirs, &sessions).is_empty());
    }

    #[test]
    fn cwd_records_are_bound_to_their_process() {
        assert_eq!(
            directories(
                "p10\nfcwd\nn/work/one\nf5\nau\nn/not/the/directory\np20\nfcwd\nn/work/two with spaces\n"
            ),
            HashMap::from([
                (10, PathBuf::from("/work/one")),
                (20, PathBuf::from("/work/two with spaces"))
            ])
        );
    }
}
