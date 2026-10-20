//! Code-signature checks on a downloaded app, done by the system's own `codesign` and `spctl`.
//!
//! An update is only installed when it is signed by the same Developer ID Team ID as the running
//! app, so a substituted asset that is merely well-formed (or signed by someone else) is refused
//! even if it got past the checksum.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CODESIGN: &str = "/usr/bin/codesign";
const SPCTL: &str = "/usr/sbin/spctl";
/// Verifying a bundle reads all of it, and Gatekeeper may consult Apple for the notarization ticket.
const TIMEOUT: Duration = Duration::from_secs(120);
const POLL: Duration = Duration::from_millis(50);

/// The Team ID the bundle at `bundle` is signed under, which must be a Developer ID team.
pub fn team_id(bundle: &Path) -> Result<String, String> {
    let output = run(Command::new(CODESIGN)
        .args(["-dv", "--verbose=4"])
        .arg(bundle))
    .map_err(|error| format!("Could not read this app's signature: {error}"))?;
    parse_team_id(&output.text).ok_or_else(|| {
        "This copy of DegenView is not signed with a Developer ID, so it cannot update itself."
            .to_owned()
    })
}

/// Fails unless `bundle`, and the login item inside it, carry a valid, notarized Developer ID
/// signature from the team `team_id`.
pub fn verify_bundle(bundle: &Path, helper: &Path, team_id: &str) -> Result<(), String> {
    let requirement = requirement(team_id)?;
    for path in [bundle, helper] {
        let output = run(Command::new(CODESIGN)
            .args(["--verify", "--deep", "--strict"])
            .arg(format!("--test-requirement={requirement}"))
            .arg(path))?;
        if !output.success {
            return Err("The download is not signed by the same developer as this app.".to_owned());
        }
    }
    // Gatekeeper's verdict also covers the notarization ticket.
    let output = run(Command::new(SPCTL)
        .args(["--assess", "--type", "execute"])
        .arg(bundle))?;
    if !output.success {
        return Err("macOS would not accept the download (it is not notarized).".to_owned());
    }
    Ok(())
}

/// A Developer ID Application signature (Apple's anchor, the Developer ID CA intermediate and the
/// Developer ID Application leaf) from `team_id`.
fn requirement(team_id: &str) -> Result<String, String> {
    if !is_team_id(team_id) {
        return Err("This app's Team ID is not valid.".to_owned());
    }
    Ok(format!(
        "=anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists \
         and certificate leaf[field.1.2.840.113635.100.6.1.13] exists \
         and certificate leaf[subject.OU] = \"{team_id}\""
    ))
}

fn is_team_id(candidate: &str) -> bool {
    candidate.len() == 10
        && candidate
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

/// `TeamIdentifier=` from `codesign -dv` output, if it names a team.
fn parse_team_id(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="))
        .map(str::trim)
        .filter(|id| is_team_id(id))
        .map(str::to_owned)
}

struct Output {
    success: bool,
    /// stdout then stderr (`codesign -d` reports on stderr).
    text: String,
}

/// Runs `command` to completion, killing it after `TIMEOUT`. Output stays small enough that the
/// pipes cannot fill while waiting.
fn run(command: &mut Command) -> Result<Output, String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let started = Instant::now();
    loop {
        if child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_some()
        {
            break;
        }
        if started.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err("checking the signature took too long".to_owned());
        }
        std::thread::sleep(POLL);
    }
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(Output {
        success: output.status.success(),
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_team_id_is_read_from_codesign_output() {
        let output = "Identifier=es.nico.degenview\nFormat=app bundle\nTeamIdentifier=AB12CD34EF\nSealed Resources version=2\n";
        assert_eq!(parse_team_id(output).as_deref(), Some("AB12CD34EF"));
    }

    #[test]
    fn an_unsigned_or_adhoc_app_has_no_team_id() {
        assert_eq!(parse_team_id("TeamIdentifier=not set\n"), None);
        assert_eq!(parse_team_id("Identifier=x\nSignature=adhoc\n"), None);
        assert_eq!(parse_team_id(""), None);
        assert_eq!(parse_team_id("TeamIdentifier=ab12cd34ef\n"), None);
    }

    #[test]
    fn only_well_formed_team_ids_reach_the_requirement() {
        assert!(
            requirement("AB12CD34EF")
                .unwrap()
                .contains("= \"AB12CD34EF\"")
        );
        for bad in [
            "",
            "AB12CD34E",
            "AB12CD34EFG",
            "ab12cd34ef",
            "AB12CD34E\"",
            "AB12 D34EF",
        ] {
            assert!(requirement(bad).is_err(), "{bad:?}");
        }
    }

    #[cfg(target_os = "macos")]
    mod macos {
        use super::*;
        use crate::service::update_archive::testing::fake_app;

        fn scratch(name: &str) -> std::path::PathBuf {
            let dir = std::env::temp_dir().join(format!(
                "degenview-code-signature-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        #[test]
        fn an_unsigned_bundle_has_no_team_id_and_does_not_verify() {
            let root = scratch("unsigned");
            let app = fake_app(&root, "x");
            let helper = app.join(crate::service::update_archive::HELPER_APP);
            assert!(team_id(&app).is_err());
            assert!(verify_bundle(&app, &helper, "AB12CD34EF").is_err());
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn an_adhoc_signed_bundle_has_no_team_id_and_does_not_verify() {
            let root = scratch("adhoc");
            let app = fake_app(&root, "x");
            let helper = app.join(crate::service::update_archive::HELPER_APP);
            let signed = Command::new(CODESIGN)
                .args(["--force", "--deep", "-s", "-"])
                .arg(&app)
                .output()
                .unwrap();
            if signed.status.success() {
                assert!(team_id(&app).is_err());
                assert!(verify_bundle(&app, &helper, "AB12CD34EF").is_err());
            }
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
