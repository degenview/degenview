//! The models a key can use, for the panel's model menu. A provider's list is whatever the key may
//! call, which for OpenAI includes embedding, speech and image models that cannot answer a chat; the
//! menu hides those, and a "type a name" row stays as the way out when the filter is wrong.

use crate::service::llm::report::ErrorReport;
use crate::service::llm::{LlmProvider, ProviderKind};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

/// Where a provider's list stands.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum ModelList {
    #[default]
    Unloaded,
    Loading,
    Ready(Vec<String>),
    Failed(ErrorReport),
}

/// Ids that are not chat models, by the words OpenAI puts in their names.
const NOT_CHAT: [&str; 12] = [
    "embedding",
    "whisper",
    "tts",
    "dall-e",
    "moderation",
    "audio",
    "realtime",
    "image",
    "transcribe",
    "davinci",
    "babbage",
    "instruct",
];

/// The ids worth offering in a chat model menu, sorted and without repeats.
pub fn usable_chat_models(kind: ProviderKind, ids: &[String]) -> Vec<String> {
    let keep = |id: &str| match kind {
        ProviderKind::OpenAi => {
            let lower = id.to_ascii_lowercase();
            let chat_family = lower.starts_with("gpt-")
                || lower.starts_with("chatgpt")
                || (lower.starts_with('o')
                    && lower.chars().nth(1).is_some_and(|c| c.is_ascii_digit()));
            chat_family && !NOT_CHAT.iter().any(|word| lower.contains(word))
        }
        // Anthropic lists only models that chat; a compatible server may call them anything.
        ProviderKind::Anthropic | ProviderKind::Compatible => true,
    };
    let mut out: Vec<String> = ids.iter().filter(|id| keep(id)).cloned().collect();
    out.sort();
    out.dedup();
    out
}

/// The models that match what is typed in the menu's filter, ignoring case.
pub fn matching<'a>(models: &'a [String], query: &str) -> Vec<&'a String> {
    let query = query.trim().to_lowercase();
    models
        .iter()
        .filter(|model| query.is_empty() || model.to_lowercase().contains(&query))
        .collect()
}

/// Asks `provider` for its models on a thread of its own; the answer arrives on the receiver.
pub fn fetch(
    provider: Box<dyn LlmProvider>,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> Receiver<Result<Vec<String>, ErrorReport>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(provider.list_models().map_err(|error| error.report()));
        wake();
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::llm::LlmError;
    use crate::service::llm::mock::MockProvider;
    use std::time::{Duration, Instant};

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn openai_keeps_chat_models_and_drops_the_rest() {
        let all = ids(&[
            "gpt-5",
            "gpt-5-mini",
            "o3",
            "o4-mini",
            "chatgpt-4o-latest",
            "text-embedding-3-large",
            "whisper-1",
            "tts-1",
            "dall-e-3",
            "gpt-4o-audio-preview",
            "gpt-4o-realtime-preview",
            "gpt-image-1",
            "omni-moderation-latest",
            "davinci-002",
            "gpt-3.5-turbo-instruct",
            "gpt-4o-transcribe",
            "somethingelse",
        ]);
        assert_eq!(
            usable_chat_models(ProviderKind::OpenAi, &all),
            ids(&["chatgpt-4o-latest", "gpt-5", "gpt-5-mini", "o3", "o4-mini"])
        );
    }

    #[test]
    fn other_providers_keep_everything_sorted_without_repeats() {
        let all = ids(&["b", "a", "b", "claude-x"]);
        assert_eq!(
            usable_chat_models(ProviderKind::Anthropic, &all),
            ids(&["a", "b", "claude-x"])
        );
        assert_eq!(
            usable_chat_models(ProviderKind::Compatible, &all),
            ids(&["a", "b", "claude-x"])
        );
        assert!(usable_chat_models(ProviderKind::OpenAi, &[]).is_empty());
    }

    #[test]
    fn the_filter_ignores_case_and_surrounding_space() {
        let models = ids(&["gpt-5", "GPT-5-mini", "o3"]);
        assert_eq!(matching(&models, "  GPT ").len(), 2);
        assert_eq!(matching(&models, "").len(), 3);
        assert!(matching(&models, "zzz").is_empty());
    }

    #[test]
    fn fetching_delivers_the_list_or_the_error() {
        let wait = |receiver: Receiver<Result<Vec<String>, ErrorReport>>| {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Ok(answer) = receiver.try_recv() {
                    return answer;
                }
                assert!(Instant::now() < deadline, "no answer");
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let ok = fetch(Box::new(MockProvider::replying(vec![])), wake.clone());
        assert_eq!(wait(ok), Ok(vec!["mock-1".to_owned()]));

        let mut failing = MockProvider::replying(vec![]);
        failing.models = Err(LlmError::auth("bad key"));
        let err = wait(fetch(Box::new(failing), wake));
        assert!(err.unwrap_err().detail.contains("bad key"));
    }
}
