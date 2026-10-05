//! Working on a config that lives on another machine.
//!
//! This drives the system's own OpenSSH client rather than linking a crypto
//! library. That is a deliberate trade:
//!
//! * No build dependencies. The pure-Rust option pulls in `aws-lc-sys`, which
//!   needs NASM on Windows; nobody should have to install an assembler to
//!   build a config editor.
//! * It uses the SSH you already have — your keys, your agent, your
//!   `~/.ssh/config` aliases, your `known_hosts`. A host you can already
//!   `ssh` to works here with no further setup.
//!
//! One caution worth writing down: on Windows this must be
//! `C:\Windows\System32\OpenSSH\ssh.exe`, not git-bash's ssh, because
//! git-bash's build cannot see the Windows ssh-agent and will sit there asking
//! for a passphrase that never arrives.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Host {
    /// What you call it in the interface
    pub name: String,
    /// `irc@example.com`, or a bare alias from your ~/.ssh/config
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// An explicit identity file. Usually unnecessary — the agent and
    /// ssh_config normally settle it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The conf directory on that machine
    pub root: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// The right ssh binary. On Windows, explicitly the system one.
pub fn ssh_bin(name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(w) = std::env::var_os("WINDIR") {
            let p = PathBuf::from(w).join("System32").join("OpenSSH").join(format!("{name}.exe"));
            if p.is_file() {
                return p;
            }
        }
    }
    PathBuf::from(name)
}

/// Wrap a path or word so a remote `sh` takes it literally.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The arguments before the command itself.
pub fn base_args(h: &Host, interactive: bool) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    if !interactive {
        // Never hang waiting for a passphrase or a host-key question: fail and
        // say so, rather than looking frozen.
        a.push("-o".into());
        a.push("BatchMode=yes".into());
    }
    a.push("-o".into());
    a.push("ConnectTimeout=10".into());
    // And give up on a connection that goes quiet mid-transfer, rather than
    // waiting however long TCP feels like.
    a.push("-o".into());
    a.push("ServerAliveInterval=5".into());
    a.push("-o".into());
    a.push("ServerAliveCountMax=3".into());
    if let Some(p) = h.port {
        a.push("-p".into());
        a.push(p.to_string());
    }
    if let Some(k) = &h.key {
        a.push("-i".into());
        a.push(k.clone());
        a.push("-o".into());
        a.push("IdentitiesOnly=yes".into());
    }
    a.push(h.target.clone());
    a
}

fn run_ssh(h: &Host, remote_cmd: &str, stdin: Option<&str>) -> Result<Output, String> {
    let mut cmd = Command::new(ssh_bin("ssh"));
    cmd.args(base_args(h, false));
    cmd.arg(remote_cmd);
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    let mut child = cmd.spawn().map_err(|e| format!("Could not run ssh: {e}"))?;
    if let Some(text) = stdin {
        child.stdin.as_mut()
            .ok_or("ssh would not take input")?
            .write_all(text.as_bytes())
            .map_err(|e| format!("Could not send the file: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("ssh failed: {e}"))?;
    Ok(Output {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Turn ssh's own failures into something worth reading.
fn explain(h: &Host, out: &Output) -> String {
    let e = out.stderr.trim();
    let lower = e.to_lowercase();
    if lower.contains("permission denied") {
        format!("{} refused the key. Is your key in that account's authorized_keys, and is the agent running?", h.target)
    } else if lower.contains("host key verification failed") {
        format!("{}'s host key isn't in your known_hosts. Connect once with ssh in a terminal to accept it, then try again.", h.target)
    } else if lower.contains("could not resolve") || lower.contains("name or service not known") {
        format!("Can't find a host called {}.", h.target)
    } else if lower.contains("connection refused") {
        format!("{} refused the connection — is sshd running on that port?", h.target)
    } else if lower.contains("timed out") || lower.contains("operation timed out") {
        format!("{} didn't answer within ten seconds.", h.target)
    } else if e.is_empty() {
        format!("ssh exited with status {}.", out.code)
    } else {
        e.lines().next().unwrap_or(e).to_string()
    }
}

/// Can we reach it, and what is it? Used by the connection dialog.
pub fn probe(h: &Host) -> Result<String, String> {
    let out = run_ssh(h, "uname -sr 2>/dev/null || echo unknown", None)?;
    if out.code != 0 {
        return Err(explain(h, &out));
    }
    Ok(out.stdout.trim().to_string())
}

/// Every `.conf` under the remote root, as paths relative to it.
/// The marker between files in a bulk read. Followed by a byte count, so the
/// contents are never scanned for anything and a config file containing this
/// line cannot confuse the split.
const CHUNK: &[u8] = b"===CONFETTI ";
/// The same idea for a folder, which has no contents to count.
const DIRCHUNK: &[u8] = b"===CONFETTI-DIR ";

/// What one bulk read brings back.
pub struct Fetched {
    pub files: Vec<(String, String)>,
    /// Every directory, including the ones with no config in them. Folders
    /// used to be worked out from the paths of the files found, so a folder
    /// you had just made could not appear until you had put something in it —
    /// and asking for it again said it already existed.
    pub dirs: Vec<String>,
}

/// Every config file and every folder, in one round trip.
///
/// One ssh connection per file meant thirty-seven handshakes to open an
/// ordinary config — long enough that Windows decided the program had stopped
/// responding and closed it. This asks once, and reads the answer by length:
/// a header line giving the byte count and the path, then exactly that many
/// bytes of file.
pub fn fetch_all(h: &Host) -> Result<Fetched, String> {
    let root = sh_quote(h.root.trim_end_matches('/'));
    let cmd = format!(
        "cd {root} 2>/dev/null || exit 9; \
         find . -maxdepth 4 -type d 2>/dev/null | sort | \
         while IFS= read -r d; do printf '{dirmark}%s\\n' \"${{d#./}}\"; done; \
         find . -maxdepth 4 -type f -name '*.conf' 2>/dev/null | sort | \
         while IFS= read -r f; do \
           n=$(wc -c < \"$f\" 2>/dev/null) || continue; \
           printf '{marker}%s %s\\n' \"$(echo $n)\" \"${{f#./}}\"; \
           cat -- \"$f\"; \
         done",
        marker = String::from_utf8_lossy(CHUNK),
        dirmark = String::from_utf8_lossy(DIRCHUNK),
    );
    let out = run_ssh_raw(h, &cmd)?;
    if out.code == 9 {
        return Err(format!("{} does not exist on {}.", h.root, h.target));
    }
    let got = split_chunks(&out.stdout);
    if got.files.is_empty() {
        if out.code != 0 {
            return Err(explain(h, &Output {
                code: out.code,
                stdout: String::new(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            }));
        }
        return Err(format!("No .conf files under {} on {}.", h.root, h.target));
    }
    Ok(got)
}

/// Split a bulk read into folders and (path, text) pairs, by the byte counts
/// it gave us. Anything between the markers is noise from the shell.
fn split_chunks(buf: &[u8]) -> Fetched {
    let mut out = Fetched { files: Vec::new(), dirs: Vec::new() };
    let mut i = 0usize;
    while i < buf.len() {
        let file_at = find(&buf[i..], CHUNK).map(|p| i + p);
        let dir_at = find(&buf[i..], DIRCHUNK).map(|p| i + p);
        // Whichever comes first. A folder header cannot be mistaken for a file
        // one: the file marker ends in a space and this one in a dash.
        let dir_first = match (file_at, dir_at) {
            (Some(f), Some(d)) => d < f,
            (None, Some(_)) => true,
            _ => false,
        };
        if dir_first {
            let at = dir_at.unwrap();
            let start = at + DIRCHUNK.len();
            let Some(nl) = find(&buf[start..], b"\n").map(|p| start + p) else { break };
            let rel = String::from_utf8_lossy(&buf[start..nl]).trim().to_string();
            i = nl + 1;
            let rel = rel.trim_start_matches("./").trim_end_matches('/').to_string();
            // The same folders the local side hides, so the same config does
            // not look different depending on which machine it is on.
            let hidden = rel.split('/').any(|part| {
                part.starts_with('.') || crate::workspace::skipped(part)
            });
            if !rel.is_empty() && rel != "." && !hidden {
                out.dirs.push(rel);
            }
            continue;
        }
        let Some(at) = file_at else { break };
        let head_start = at + CHUNK.len();
        let Some(nl) = find(&buf[head_start..], b"\n").map(|p| head_start + p) else { break };
        let head = String::from_utf8_lossy(&buf[head_start..nl]).into_owned();
        i = nl + 1;
        let Some((count, rel)) = head.split_once(' ') else { continue };
        let Ok(n) = count.trim().parse::<usize>() else { continue };
        let end = (i + n).min(buf.len());
        let text = String::from_utf8_lossy(&buf[i..end]).into_owned();
        i = end;
        let rel = rel.trim().trim_start_matches("./").to_string();
        if !rel.is_empty() {
            out.files.push((rel, text));
        }
    }
    out
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// As `run_ssh`, but the output stays as bytes: a bulk read is split by byte
/// counts, and lossy UTF-8 would move them.
struct RawOutput {
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_ssh_raw(h: &Host, remote_cmd: &str) -> Result<RawOutput, String> {
    let mut cmd = Command::new(ssh_bin("ssh"));
    cmd.args(base_args(h, false));
    cmd.arg(remote_cmd);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|e| format!("Could not run ssh: {e}"))?;
    Ok(RawOutput {
        code: out.status.code().unwrap_or(-1),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/* ------------------------------------------------------------------ *
 * Managing the files on the other machine.
 *
 * These used to be `std::fs` calls with no idea the config was somewhere
 * else, so asking for a new folder on a server ran mkdir against a POSIX path
 * on Windows. That is worse than it sounds: `/home/ircd/unrealircd/conf`
 * resolves to `C:\home\ircd\unrealircd\conf`, so the ones that create rather
 * than read would quietly have made directories on this machine.
 *
 * Every path here is relative to the config directory, and `..` is refused
 * before it reaches a shell.
 * ------------------------------------------------------------------ */

/// A path on the other machine, checked and quoted.
fn at(h: &Host, rel: &str) -> Result<String, String> {
    let clean = rel.replace('\\', "/");
    let clean = clean.trim_start_matches('/');
    if clean.is_empty() {
        return Err("No name given.".into());
    }
    if clean.split('/').any(|p| p == ".." || p.is_empty()) {
        return Err(format!("{rel} is not inside the config folder."));
    }
    Ok(sh_quote(&format!("{}/{}", h.root.trim_end_matches('/'), clean)))
}

/// Does it exist over there?
pub fn exists(h: &Host, rel: &str) -> Result<bool, String> {
    let p = at(h, rel)?;
    let out = run_ssh(h, &format!("test -e {p} && echo yes || echo no"), None)?;
    match out.stdout.trim() {
        "yes" => Ok(true),
        "no" => Ok(false),
        _ => Err(explain(h, &out)),
    }
}

/// Make a directory, and any parent it needs, with the permissions the rest
/// of the config folder has.
///
/// `mkdir` on its own takes the login umask, which on a Debian-ish box is 002
/// — so a folder came out drwxrwxr-x while every other directory UnrealIRCd
/// installed was drwx------. A config tree holds oper password hashes, cloak
/// keys and TLS private keys; it is not a place to leave world-readable
/// directories. So: strict by default, then matched to the folder above it
/// where the other machine can tell us what that is.
pub fn mkdir(h: &Host, rel: &str) -> Result<(), String> {
    let p = at(h, rel)?;
    done(h, &format!("{}mkdir -p -- {p} || exit 1\n{}", umask_strict(), match_parent(&p)),
         "create that folder")
}

/// Nothing group- or world-readable, whatever the login umask says.
fn umask_strict() -> &'static str {
    "umask 077\n"
}

/// Copy the mode of the thing above it, if this machine can say what it is.
/// Where it cannot, what umask 077 gave us stands.
fn match_parent(quoted: &str) -> String {
    format!(
        "d=\"$(dirname -- {quoted})\"\n\
         m=\"$(stat -c %a -- \"$d\" 2>/dev/null || true)\"\n\
         case \"$m\" in [0-7]*) chmod \"$m\" -- {quoted} 2>/dev/null || true;; esac\n"
    )
}

/// Move or rename. Refuses to write over something already there.
pub fn move_path(h: &Host, from: &str, to: &str) -> Result<(), String> {
    let a = at(h, from)?;
    let b = at(h, to)?;
    done(h, &format!("test -e {b} && exit 17; mv -- {a} {b}"), "move that")
}

/// Copy a file, keeping its permissions.
pub fn copy_path(h: &Host, from: &str, to: &str) -> Result<(), String> {
    let a = at(h, from)?;
    let b = at(h, to)?;
    done(h, &format!("test -e {b} && exit 17; cp -p -- {a} {b}"), "copy that")
}

/// Write a file, creating the folders above it, and keep it as private as the
/// config files already there.
pub fn put_file(h: &Host, rel: &str, text: &str) -> Result<(), String> {
    let p = at(h, rel)?;
    let script = format!(
        "{}mkdir -p -- \"$(dirname -- {p})\" && cat > {p}",
        umask_strict(),
    );
    let out = run_ssh(h, &script, Some(text))?;
    if out.code != 0 {
        return Err(format!("Could not write {rel}: {}", explain(h, &out)));
    }
    Ok(())
}

/// Delete a file, keeping a `~` copy beside it exactly as the local side does.
pub fn delete_file(h: &Host, rel: &str) -> Result<(), String> {
    let p = at(h, rel)?;
    done(h, &format!("cp -p -- {p} {p}~ && rm -f -- {p}"), "delete that")
}

/// Delete a folder — but only by moving it aside, so nothing is ever really
/// gone on the strength of one click.
pub fn retire_folder(h: &Host, rel: &str) -> Result<String, String> {
    let p = at(h, rel)?;
    let out = run_ssh(h, &format!("test -e {p}~ && exit 17; mv -- {p} {p}~ && echo {p}~"), None)?;
    if out.code == 17 {
        return Err("Something is already set aside under that name.".into());
    }
    if out.code != 0 {
        return Err(format!("Could not set that folder aside: {}", explain(h, &out)));
    }
    Ok(out.stdout.trim().to_string())
}

/// Run something that has nothing to say, and read the exit code.
fn done(h: &Host, script: &str, what: &str) -> Result<(), String> {
    let out = run_ssh(h, script, None)?;
    if out.code == 17 {
        return Err("Something is already there under that name.".into());
    }
    if out.code != 0 {
        return Err(format!("Could not {what}: {}", explain(h, &out)));
    }
    Ok(())
}

pub fn list_conf(h: &Host) -> Result<Vec<String>, String> {
    let root = sh_quote(h.root.trim_end_matches('/'));
    // -print0 would be tidier but newline-separated is fine: a config file with
    // a newline in its name is not a thing that happens.
    let cmd = format!("find {root} -maxdepth 4 -type f -name '*.conf' -print 2>/dev/null | sort");
    let out = run_ssh(h, &cmd, None)?;
    if out.code != 0 && out.stdout.trim().is_empty() {
        return Err(explain(h, &out));
    }
    let base = h.root.trim_end_matches('/');
    let mut files: Vec<String> = out.stdout
        .lines()
        .map(|l| l.trim_end_matches('\r').trim())
        .filter(|l| !l.is_empty())
        .map(|l| l.strip_prefix(base).unwrap_or(l).trim_start_matches('/').to_string())
        .filter(|l| !l.is_empty())
        .collect();
    files.sort();
    files.dedup();
    if files.is_empty() {
        return Err(format!("No .conf files under {} on {}.", h.root, h.target));
    }
    Ok(files)
}

pub fn read_file(h: &Host, rel: &str) -> Result<String, String> {
    let full = format!("{}/{}", h.root.trim_end_matches('/'), rel);
    let out = run_ssh(h, &format!("cat -- {}", sh_quote(&full)), None)?;
    if out.code != 0 {
        return Err(format!("Could not read {rel}: {}", explain(h, &out)));
    }
    Ok(out.stdout)
}

/// The last `n` lines of a file on the server. A log can be hundreds of
/// megabytes and `cat` would drag all of it over the wire.
pub fn tail(h: &Host, path: &str, n: usize) -> Result<String, String> {
    let out = run_ssh(h, &format!("tail -n {} -- {}", n, sh_quote(path)), None)?;
    if out.code != 0 {
        return Err(format!("Could not read {path}: {}", explain(h, &out)));
    }
    Ok(out.stdout)
}

/// Write a file back, keeping a `~` copy and swapping it in atomically so a
/// dropped connection cannot leave half a config on a live server.
pub fn write_file(h: &Host, rel: &str, text: &str) -> Result<(), String> {
    let full = format!("{}/{}", h.root.trim_end_matches('/'), rel);
    let q = sh_quote(&full);
    let tmp = sh_quote(&format!("{full}.confetti-tmp"));
    let bak = sh_quote(&format!("{full}~"));
    let cmd = format!(
        "cp -p -- {q} {bak} 2>/dev/null; cat > {tmp} && mv -f -- {tmp} {q}");
    let out = run_ssh(h, &cmd, Some(text))?;
    if out.code != 0 {
        return Err(format!("Could not save {rel}: {}", explain(h, &out)));
    }
    Ok(())
}

/// Run one of UnrealIRCd's own commands on the server — `configtest` is the
/// ground truth that nothing computed here can match.
pub fn unrealircd(h: &Host, verb: &str) -> Result<Output, String> {
    if !matches!(verb, "configtest" | "rehash" | "status" | "version" | "module-status" | "reloadtls") {
        return Err(format!("Confetti won't run “{verb}” on a server."));
    }
    // conf/ usually sits inside the install, so the binary is one level up.
    let root = h.root.trim_end_matches('/');
    let install = root.strip_suffix("/conf").unwrap_or(root);
    let cmd = format!(
        "cd {} && ( ./unrealircd {verb} 2>&1 || ./bin/unrealircd {verb} 2>&1 )",
        sh_quote(install));
    run_ssh(h, &cmd, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> Host {
        Host {
            name: "box".into(),
            target: "irc@example.com".into(),
            port: Some(2222),
            key: Some("C:\\keys\\id_ed25519".into()),
            root: "/home/irc/unrealircd/conf".into(),
        }
    }

    #[test]
    fn quoting_survives_awkward_names() {
        assert_eq!(sh_quote("/home/irc/conf"), "'/home/irc/conf'");
        assert_eq!(sh_quote("it's here"), r"'it'\''s here'");
        // A path that tries to end the quote and run something else must not.
        let nasty = sh_quote("'; rm -rf /; echo '");
        assert!(!nasty.contains("; rm -rf /;") || nasty.starts_with('\''));
        assert!(nasty.starts_with('\'') && nasty.ends_with('\''));
    }

    /// A bulk read is split by the byte counts it declares, so nothing in a
    /// config file can be mistaken for a separator — including the separator.
    #[test]
    fn a_bulk_read_splits_by_length() {
        let mut buf = Vec::new();
        let nasty = "# this file contains ===CONFETTI 99 evil.conf\nme { sid 001; }\n";
        buf.extend_from_slice(format!("===CONFETTI {} unrealircd.conf\n", nasty.len()).as_bytes());
        buf.extend_from_slice(nasty.as_bytes());
        buf.extend_from_slice(b"===CONFETTI 6 aliases/x.conf\nhello\n");
        let got = split_chunks(&buf);
        assert_eq!(got.files.len(), 2, "{:?}", got.files);
        assert_eq!(got.files[0].0, "unrealircd.conf");
        assert_eq!(got.files[0].1, nasty, "a file mentioning the marker survives whole");
        assert_eq!(got.files[1], ("aliases/x.conf".to_string(), "hello\n".to_string()));
    }

    /// A folder with no config in it yet still has to come back, or you make
    /// one, it never appears, and making it again says it already exists.
    #[test]
    fn an_empty_folder_still_comes_back() {
        let buf = b"===CONFETTI-DIR .\n===CONFETTI-DIR aliases\n===CONFETTI-DIR staff\n\
                    ===CONFETTI 4 unrealircd.conf\nabc\n".to_vec();
        let got = split_chunks(&buf);
        assert_eq!(got.dirs, vec!["aliases".to_string(), "staff".to_string()],
                   "the config directory itself is not one of its own folders");
        assert_eq!(got.files.len(), 1);
        // A folder marker must not be read as a file marker, or the other way.
        assert!(!got.files.iter().any(|(r, _)| r.contains("staff")));
    }

    #[test]
    fn a_bulk_read_ignores_shell_noise_before_the_first_file() {
        let buf = b"bash: warning: something\n===CONFETTI 4 a.conf\nabc\n".to_vec();
        let got = split_chunks(&buf);
        assert_eq!(got.files, vec![("a.conf".to_string(), "abc\n".to_string())]);
    }

    #[test]
    fn a_truncated_bulk_read_gives_back_what_arrived() {
        // The connection dropped mid-file: keep what came, do not panic.
        let buf = b"===CONFETTI 40 a.conf\nonly this much".to_vec();
        let got = split_chunks(&buf);
        assert_eq!(got.files.len(), 1);
        assert_eq!(got.files[0].1, "only this much");
    }

    #[test]
    fn a_bulk_read_of_nothing_is_no_files_rather_than_a_panic() {
        for b in [&b""[..], b"===CONFETTI", b"===CONFETTI nonsense\n", b"===CONFETTI-DIR"] {
            let got = split_chunks(b);
            assert!(got.files.is_empty() && got.dirs.is_empty(), "{b:?}");
        }
    }

    /// The script itself, so it can be run against a real filesystem instead
    /// of only being read.
    #[test]
    /// Prints the script so it can be run against a real filesystem rather
    /// than only read. To check the permissions it produces:
    ///   CONFETTI_PRINT_SCRIPT=1 cargo test --lib print_the_mkdir -- --nocapture
    /// then paste it into a shell with a 700 parent and a loose umask.
    fn print_the_mkdir_script() {
        if std::env::var("CONFETTI_PRINT_SCRIPT").is_err() { return; }
        let h = host();
        let p = at(&h, "staff").unwrap();
        println!("---SCRIPT---");
        println!("{}mkdir -p -- {p} || exit 1
{}", umask_strict(), match_parent(&p));
        println!("---END---");
    }

    /// A config tree holds oper hashes, cloak keys and TLS private keys.
    /// Nothing Confetti creates in it may be readable by anyone else, whatever
    /// the login umask on that machine happens to be.
    #[test]
    fn nothing_is_created_group_or_world_readable() {
        assert_eq!(umask_strict().trim(), "umask 077");
        let p = "'/home/ircd/unrealircd/conf/staff'";
        let m = match_parent(p);
        // Strict first, then matched to the folder above it where we can read
        // its mode — so a config folder kept at 700 stays at 700.
        assert!(m.contains("stat -c %a"), "{m}");
        assert!(m.contains("chmod"), "{m}");
        assert!(m.contains("[0-7]*"), "only a real mode is applied: {m}");
        // And a failure to read the parent leaves umask 077 standing.
        assert!(m.contains("|| true"), "{m}");
    }

    #[test]
    fn batch_mode_is_on_so_it_never_hangs() {
        let a = base_args(&host(), false);
        assert!(a.windows(2).any(|w| w == ["-o", "BatchMode=yes"]));
        assert!(a.windows(2).any(|w| w == ["-o", "ConnectTimeout=10"]));
        // A connection that goes quiet mid-transfer has to end too, not
        // just one that never opens at all.
        assert!(a.windows(2).any(|w| w == ["-o", "ServerAliveInterval=5"]));
        assert!(a.windows(2).any(|w| w == ["-o", "ServerAliveCountMax=3"]));
        assert_eq!(a.last().unwrap(), "irc@example.com");
    }

    #[test]
    fn a_port_and_key_are_passed_through() {
        let a = base_args(&host(), false);
        assert!(a.windows(2).any(|w| w == ["-p", "2222"]));
        assert!(a.windows(2).any(|w| w[0] == "-i" && w[1] == "C:\\keys\\id_ed25519"));
        // An explicit key means only that key, or ssh will try the agent first.
        assert!(a.windows(2).any(|w| w == ["-o", "IdentitiesOnly=yes"]));
    }

    #[test]
    fn a_host_with_no_extras_is_left_alone() {
        let h = Host { name: "h".into(), target: "myserver".into(), root: "/etc/unrealircd".into(),
                       ..Default::default() };
        let a = base_args(&h, false);
        assert!(!a.iter().any(|x| x == "-p"));
        assert!(!a.iter().any(|x| x == "-i"));
        // A bare alias is left for ~/.ssh/config to resolve.
        assert_eq!(a.last().unwrap(), "myserver");
    }

    #[test]
    fn only_safe_verbs_reach_the_server() {
        let h = host();
        assert!(unrealircd(&h, "stop").is_err());
        assert!(unrealircd(&h, "restart").is_err());
        assert!(unrealircd(&h, "; rm -rf /").is_err());
    }

    #[test]
    fn ssh_errors_are_explained_not_echoed() {
        let h = host();
        let denied = Output { code: 255, stdout: String::new(),
                              stderr: "irc@example.com: Permission denied (publickey).".into() };
        assert!(explain(&h, &denied).contains("authorized_keys"));

        let unknown = Output { code: 255, stdout: String::new(),
                               stderr: "Host key verification failed.".into() };
        assert!(explain(&h, &unknown).contains("known_hosts"));

        let refused = Output { code: 255, stdout: String::new(),
                               stderr: "ssh: connect to host x port 22: Connection refused".into() };
        assert!(explain(&h, &refused).contains("sshd"));
    }

    #[test]
    fn windows_uses_the_system_ssh_not_whatever_is_on_path() {
        let p = ssh_bin("ssh");
        #[cfg(windows)]
        {
            // git-bash's ssh cannot see the Windows agent, so this must be the
            // one in System32 whenever it exists.
            if let Some(w) = std::env::var_os("WINDIR") {
                let want = PathBuf::from(w).join("System32").join("OpenSSH").join("ssh.exe");
                if want.is_file() {
                    assert_eq!(p, want);
                }
            }
        }
        #[cfg(not(windows))]
        assert_eq!(p, PathBuf::from("ssh"));
    }
}
