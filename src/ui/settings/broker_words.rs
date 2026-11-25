//! What a broker's descriptor says, in the language in use.
//!
//! `degenview-brokers` is a separate crate: it writes English and knows nothing of the app's
//! catalogs, so the setup steps, the permission note and the field names it carries are put through
//! `tr!` here, one broker at a time. A broker this file does not know shows the crate's own English,
//! which never breaks a screen; the tests compare the English written here with each known
//! descriptor, so a crate update that rewords a step fails there and is not shipped as a translation
//! of the old text.

use crate::i18n::tr;
use degenview_brokers::coinbase;
use degenview_brokers::{CredentialField, Descriptor};

/// How to create a credential, one entry per step.
pub fn setup_steps(descriptor: &Descriptor) -> Vec<String> {
    let steps: Vec<&str> = if descriptor.id == coinbase::ID {
        vec![
            tr!("Open the Coinbase Developer Platform (portal.cdp.coinbase.com) and sign in."),
            tr!("Go to API keys and create a Secret API key for the portfolio you want to trade."),
            tr!("Choose the ECDSA signature algorithm if you are offered one."),
            tr!("Tick View, and Trade only if you want to place orders from DegenView."),
            tr!("Do not tick Transfer. DegenView never moves funds and does not need it."),
            tr!("Copy the key name and the private key (shown once) into the fields below."),
        ]
    } else if descriptor.id == coinbase::SANDBOX_ID {
        vec![
            tr!("There is nothing to create: Coinbase's sandbox needs no API key."),
            tr!(
                "Its answers are fixed examples. The balances, orders and fills do not change when you place, edit or cancel an order, and it has no live prices."
            ),
            tr!(
                "Use it to try the order ticket and its previews without a real account. Nothing here reaches your Coinbase account."
            ),
        ]
    } else {
        descriptor.setup_steps.to_vec()
    };
    steps.into_iter().map(str::to_owned).collect()
}

/// What to tick (and not tick) when creating the credential.
pub fn permission_note(descriptor: &Descriptor) -> &'static str {
    if descriptor.id == coinbase::ID {
        tr!(
            "View lets DegenView read balances and orders. Trade lets it place and cancel orders. Transfer is never needed."
        )
    } else if descriptor.id == coinbase::SANDBOX_ID {
        tr!("No key is used, so there is nothing to grant. Press Connect.")
    } else {
        descriptor.permission_note
    }
}

/// The title of a credential field.
pub fn field_label(descriptor: &Descriptor, field: &CredentialField) -> &'static str {
    if descriptor.id == coinbase::ID {
        match field.key {
            coinbase::FIELD_NAME => return tr!("API key name"),
            coinbase::FIELD_PRIVATE_KEY => return tr!("Private key"),
            _ => {}
        }
    }
    field.label
}

/// The example shown in an empty credential field. Ids and paths are not text and stay as the
/// crate writes them.
pub fn field_hint(descriptor: &Descriptor, field: &CredentialField) -> &'static str {
    if descriptor.id == coinbase::ID && field.key == coinbase::FIELD_PRIVATE_KEY {
        return tr!("-----BEGIN EC PRIVATE KEY----- … (or a base64 Ed25519 key)");
    }
    field.hint
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Locale, with_locale};

    fn known() -> [&'static Descriptor; 2] {
        [&coinbase::DESCRIPTOR, &coinbase::SANDBOX_DESCRIPTOR]
    }

    /// Written in English here, so the crate rewording a step or a note fails this test.
    #[test]
    fn the_english_written_here_is_the_crates_own() {
        with_locale(Locale::ENGLISH, || {
            for descriptor in known() {
                assert_eq!(
                    setup_steps(descriptor),
                    descriptor
                        .setup_steps
                        .iter()
                        .map(|step| (*step).to_owned())
                        .collect::<Vec<_>>(),
                    "{}",
                    descriptor.id
                );
                assert_eq!(
                    permission_note(descriptor),
                    descriptor.permission_note,
                    "{}",
                    descriptor.id
                );
                for field in descriptor.credential_fields {
                    assert_eq!(field_label(descriptor, field), field.label);
                    assert_eq!(field_hint(descriptor, field), field.hint);
                }
            }
        });
    }

    #[test]
    fn a_broker_this_file_does_not_know_shows_what_its_crate_wrote() {
        let mut other = coinbase::DESCRIPTOR;
        other.id = degenview_brokers::BrokerId::new("acme");
        with_locale(Locale::PSEUDO, || {
            assert_eq!(setup_steps(&other).len(), other.setup_steps.len());
            assert_eq!(setup_steps(&other)[0], other.setup_steps[0]);
            assert_eq!(permission_note(&other), other.permission_note);
        });
    }

    #[test]
    fn the_words_follow_the_language() {
        with_locale(Locale::PSEUDO, || {
            for descriptor in known() {
                assert_ne!(
                    permission_note(descriptor),
                    descriptor.permission_note,
                    "{}",
                    descriptor.id
                );
            }
        });
    }
}
