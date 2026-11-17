//! Short system sounds for events the user should hear, played through what the platform already
//! has: no audio crate, no output stream held open.
//!
//! [`SystemSounder::play`] returns at once; the sound is started off the calling thread (or
//! asynchronously by the system), so a frame never waits on an audio device. A platform with no way
//! to play one, or a sound that fails to start, is silent: a missing chime is never an error the
//! user has to read.

/// What happened, which decides the sound.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Sound {
    /// A paper order was filled.
    OrderFilled,
}

/// Plays sounds. A trait so the code deciding when to play can be tested without an audio device.
pub trait Sounder: Send + Sync {
    fn play(&self, sound: Sound);
}

/// The platform's own sound facility.
pub struct SystemSounder;

impl Sounder for SystemSounder {
    fn play(&self, sound: Sound) {
        platform::play(sound);
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::Sound;
    use objc2_foundation::ns_string;

    fn name(sound: Sound) -> &'static objc2_foundation::NSString {
        match sound {
            Sound::OrderFilled => ns_string!("Glass"),
        }
    }

    pub fn play(sound: Sound) {
        // `NSSound` keeps a named sound cached; playing starts it and returns.
        std::thread::spawn(move || {
            if let Some(sound) = objc2_app_kit::NSSound::soundNamed(name(sound)) {
                // A sound still playing from the last fill restarts rather than being ignored.
                sound.stop();
                sound.play();
            }
        });
    }
}

#[cfg(windows)]
mod platform {
    use super::Sound;

    /// A name from the user's Sound control panel, so it follows their scheme.
    fn alias(sound: Sound) -> &'static str {
        match sound {
            Sound::OrderFilled => "SystemNotification",
        }
    }

    pub fn play(sound: Sound) {
        use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT};
        let alias: Vec<u16> = alias(sound).encode_utf16().chain(Some(0)).collect();
        // SAFETY: `alias` is a NUL-terminated UTF-16 string that outlives the call (SND_ASYNC
        // copies an alias before returning); no module handle is needed for an alias.
        unsafe {
            PlaySoundW(
                alias.as_ptr(),
                std::ptr::null_mut(),
                SND_ALIAS | SND_ASYNC | SND_NODEFAULT,
            );
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::Sound;
    use std::process::{Command, Stdio};

    /// An id from the freedesktop sound theme.
    fn theme_id(sound: Sound) -> &'static str {
        match sound {
            Sound::OrderFilled => "message",
        }
    }

    /// Asks `canberra-gtk-play` (present on most desktops) and stays silent where it is not, as in
    /// a Flatpak sandbox without it.
    pub fn play(sound: Sound) {
        std::thread::spawn(move || {
            let child = Command::new("canberra-gtk-play")
                .args(["-i", theme_id(sound)])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            if let Ok(mut child) = child {
                // Reaped here so the finished player does not linger as a zombie.
                let _ = child.wait();
            }
        });
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod platform {
    use super::Sound;

    pub fn play(_: Sound) {}
}
