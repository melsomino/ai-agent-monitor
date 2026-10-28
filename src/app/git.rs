//! Read current repository metadata without changing the working tree.
use crate::app::process::output_with_timeout;
use std::{path::Path, process::Command, time::Duration};

pub fn branch(directory: &Path) -> Option<String> {
    if !directory.is_dir() {
        return None;
    }
    let read = |args: &[&str]| {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(directory)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0");
        let (status, output, _) = output_with_timeout(&mut command, Duration::from_secs(2)).ok()?;
        (status == Some(0))
            .then(|| output.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    read(&["symbolic-ref", "--quiet", "--short", "HEAD"]).or_else(|| {
        read(&["rev-parse", "--short", "HEAD"]).map(|commit| format!("HEAD · {commit}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn git(directory: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(["-c", "core.hooksPath=/dev/null"])
            .arg("-C")
            .arg(directory)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn reads_unborn_branches_worktrees_and_detached_heads_from_the_given_directory() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        assert_eq!(branch(&repo), None);
        git(&repo, &["init", "-q", "-b", "main"]);
        assert_eq!(branch(&repo).as_deref(), Some("main"));
        git(
            &repo,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-qm",
                "Initial",
            ],
        );
        let worktree = temp.path().join("worktree");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature/monitor",
                worktree.to_str().unwrap(),
            ],
        );
        assert!(worktree.join(".git").is_file());
        assert_eq!(branch(&worktree).as_deref(), Some("feature/monitor"));
        assert_eq!(branch(&repo).as_deref(), Some("main"));
        git(&worktree, &["checkout", "--detach", "-q"]);
        assert!(branch(&worktree).unwrap().starts_with("HEAD · "));
        assert_eq!(branch(&temp.path().join("missing")), None);
    }
}
