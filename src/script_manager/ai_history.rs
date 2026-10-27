//! Saved conversations: what was asked about each script and what the assistant answered, kept
//! across restarts. One local `setting` (`ai.history`) holds them all; it names the user's own
//! scripts and prompts, so it never syncs.
//!
//! Everything is capped so it cannot grow without bound: turns per conversation, characters per
//! turn and per conversation, conversations kept (the most recently used), and a total. Trimming
//! drops the oldest turns first and never leaves a conversation starting with a reply. Writes are
//! read-modify-write against the store, so a conversation cleared from Settings is not brought back
//! by a copy held in memory.

use super::ai_prompt::{Mode, Turn};
use crate::store::Store;
use serde::{Deserialize, Serialize};

pub const KEY: &str = "ai.history";

const MAX_CONVERSATIONS: usize = 30;
const MAX_TURNS: usize = 40;
const MAX_TURN_CHARS: usize = 40_000;
const MAX_CONVERSATION_CHARS: usize = 120_000;
const MAX_TOTAL_CHARS: usize = 1_000_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "role", rename_all = "lowercase")]
enum Stored {
    User { text: String },
    Assistant { text: String, mode: Mode },
}

impl Stored {
    fn text(&self) -> &str {
        match self {
            Self::User { text } | Self::Assistant { text, .. } => text,
        }
    }

    fn text_mut(&mut self) -> &mut String {
        match self {
            Self::User { text } | Self::Assistant { text, .. } => text,
        }
    }
}

impl From<&Turn> for Stored {
    fn from(turn: &Turn) -> Self {
        match turn {
            Turn::User(text) => Self::User { text: text.clone() },
            Turn::Assistant { text, mode } => Self::Assistant {
                text: text.clone(),
                mode: *mode,
            },
        }
    }
}

impl From<Stored> for Turn {
    fn from(stored: Stored) -> Self {
        match stored {
            Stored::User { text } => Self::User(text),
            Stored::Assistant { text, mode } => Self::Assistant { text, mode },
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Conversation {
    script_id: String,
    used_ms: i64,
    turns: Vec<Stored>,
}

impl Conversation {
    fn chars(&self) -> usize {
        self.turns.iter().map(|t| t.text().chars().count()).sum()
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct History {
    conversations: Vec<Conversation>,
}

/// `text` cut to at most `max` characters, on a character boundary.
fn clip(text: &mut String, max: usize) {
    if let Some((index, _)) = text.char_indices().nth(max) {
        text.truncate(index);
        text.push('…');
    }
}

impl History {
    /// Whatever is saved, or nothing: a history that cannot be read is not worth failing over.
    pub fn load(store: &Store) -> Self {
        let mut history: Self = store.setting(KEY).unwrap_or_default();
        // Drops anything empty or over a cap, which a hand-edited or older value could hold.
        history.trim();
        history
    }

    pub fn save(&self, store: &Store) -> Result<(), String> {
        store
            .set_setting(KEY, self)
            .map_err(|error| error.to_string())
    }

    pub fn is_empty(&self) -> bool {
        self.conversations.is_empty()
    }

    /// The saved turns about `script_id`, oldest first.
    pub fn get(&self, script_id: &str) -> Vec<Turn> {
        self.conversations
            .iter()
            .find(|c| c.script_id == script_id)
            .map(|c| c.turns.iter().cloned().map(Turn::from).collect())
            .unwrap_or_default()
    }

    /// Replaces the conversation about `script_id`. No turns removes it.
    pub fn put(&mut self, script_id: &str, turns: &[Turn], now_ms: i64) {
        self.conversations.retain(|c| c.script_id != script_id);
        if !turns.is_empty() {
            self.conversations.push(Conversation {
                script_id: script_id.to_owned(),
                used_ms: now_ms,
                turns: turns.iter().map(Stored::from).collect(),
            });
        }
        self.trim();
    }

    pub fn remove(&mut self, script_id: &str) {
        self.conversations.retain(|c| c.script_id != script_id);
    }

    /// Forgets conversations about scripts that are gone.
    pub fn retain_scripts(&mut self, exists: &dyn Fn(&str) -> bool) {
        self.conversations.retain(|c| exists(&c.script_id));
    }

    pub fn clear(&mut self) {
        self.conversations.clear();
    }

    fn trim(&mut self) {
        for conversation in &mut self.conversations {
            for turn in &mut conversation.turns {
                clip(turn.text_mut(), MAX_TURN_CHARS);
            }
            while conversation.turns.len() > MAX_TURNS
                || conversation.chars() > MAX_CONVERSATION_CHARS
            {
                if conversation.turns.is_empty() {
                    break;
                }
                conversation.turns.remove(0);
            }
            // A conversation opens with the user, as a request to a provider must.
            while matches!(conversation.turns.first(), Some(Stored::Assistant { .. })) {
                conversation.turns.remove(0);
            }
        }
        self.conversations.retain(|c| !c.turns.is_empty());
        // Most recently used first; the least recent go when there are too many or too much.
        self.conversations
            .sort_by_key(|c| std::cmp::Reverse(c.used_ms));
        self.conversations.truncate(MAX_CONVERSATIONS);
        let mut total = 0;
        let mut keep = 0;
        for conversation in &self.conversations {
            total += conversation.chars();
            if total > MAX_TOTAL_CHARS {
                break;
            }
            keep += 1;
        }
        self.conversations.truncate(keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange(n: usize) -> Vec<Turn> {
        (0..n)
            .flat_map(|i| {
                [
                    Turn::User(format!("question {i}")),
                    Turn::Assistant {
                        text: format!("answer {i}"),
                        mode: Mode::Plan,
                    },
                ]
            })
            .collect()
    }

    #[test]
    fn a_conversation_round_trips_with_its_modes() {
        let mut history = History::default();
        let turns = vec![
            Turn::User("a".into()),
            Turn::Assistant {
                text: "b".into(),
                mode: Mode::Edit,
            },
        ];
        history.put("script-1", &turns, 10);
        assert_eq!(history.get("script-1"), turns);
        assert!(history.get("other").is_empty());
        let json = serde_json::to_string(&history).unwrap();
        let back: History = serde_json::from_str(&json).unwrap();
        assert_eq!(back.get("script-1"), turns);
    }

    #[test]
    fn putting_nothing_removes_the_conversation() {
        let mut history = History::default();
        history.put("s", &exchange(1), 1);
        history.put("s", &[], 2);
        assert!(history.is_empty());
        history.put("s", &exchange(1), 3);
        history.remove("s");
        assert!(history.is_empty());
    }

    #[test]
    fn putting_again_replaces_rather_than_duplicates() {
        let mut history = History::default();
        history.put("s", &exchange(1), 1);
        history.put("s", &exchange(2), 2);
        assert_eq!(history.get("s").len(), 4);
        assert_eq!(history.conversations.len(), 1);
    }

    #[test]
    fn a_long_conversation_keeps_its_latest_turns_and_starts_with_the_user() {
        let mut history = History::default();
        history.put("s", &exchange(60), 1);
        let turns = history.get("s");
        assert!(turns.len() <= MAX_TURNS);
        assert!(matches!(turns[0], Turn::User(_)));
        assert!(matches!(
            turns.last(),
            Some(Turn::Assistant { text, .. }) if text == "answer 59"
        ));
    }

    #[test]
    fn a_huge_turn_is_clipped_on_a_character_boundary() {
        let mut history = History::default();
        let big = "日".repeat(MAX_TURN_CHARS + 500);
        history.put("s", &[Turn::User(big)], 1);
        let Turn::User(text) = &history.get("s")[0] else {
            panic!("a user turn")
        };
        assert_eq!(text.chars().count(), MAX_TURN_CHARS + 1);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn a_conversation_over_the_character_cap_drops_its_oldest_turns() {
        let mut history = History::default();
        let chunk = "x".repeat(30_000);
        let turns: Vec<Turn> = (0..10)
            .flat_map(|_| {
                [
                    Turn::User(chunk.clone()),
                    Turn::Assistant {
                        text: chunk.clone(),
                        mode: Mode::Plan,
                    },
                ]
            })
            .collect();
        history.put("s", &turns, 1);
        let saved = history.get("s");
        let chars: usize = saved
            .iter()
            .map(|t| match t {
                Turn::User(text) | Turn::Assistant { text, .. } => text.chars().count(),
            })
            .sum();
        assert!(chars <= MAX_CONVERSATION_CHARS);
        assert!(matches!(saved[0], Turn::User(_)));
    }

    #[test]
    fn only_the_most_recently_used_conversations_are_kept() {
        let mut history = History::default();
        for i in 0..40 {
            history.put(&format!("s{i}"), &exchange(1), i as i64);
        }
        assert_eq!(history.conversations.len(), MAX_CONVERSATIONS);
        assert!(history.get("s39").len() == 2, "the newest stays");
        assert!(history.get("s0").is_empty(), "the oldest goes");
    }

    #[test]
    fn the_total_is_capped_by_dropping_the_least_recent() {
        let mut history = History::default();
        let chunk = "y".repeat(35_000);
        for i in 0..20 {
            history.put(
                &format!("s{i}"),
                &[
                    Turn::User(chunk.clone()),
                    Turn::Assistant {
                        text: chunk.clone(),
                        mode: Mode::Plan,
                    },
                ],
                i as i64,
            );
        }
        let total: usize = history.conversations.iter().map(Conversation::chars).sum();
        assert!(total <= MAX_TOTAL_CHARS, "{total}");
        assert!(!history.get("s19").is_empty(), "the newest survives");
    }

    #[test]
    fn scripts_that_are_gone_are_forgotten() {
        let mut history = History::default();
        history.put("keep", &exchange(1), 1);
        history.put("gone", &exchange(1), 2);
        history.retain_scripts(&|id| id == "keep");
        assert!(history.get("gone").is_empty());
        assert!(!history.get("keep").is_empty());
        history.clear();
        assert!(history.is_empty());
    }

    #[test]
    fn it_saves_loads_and_never_syncs() {
        let store = Store::open_in_memory().unwrap();
        assert!(History::load(&store).is_empty());
        let mut history = History::default();
        history.put("s", &exchange(2), 5);
        history.save(&store).unwrap();
        assert_eq!(History::load(&store).get("s"), history.get("s"));
        assert_eq!(
            crate::sync::policy::setting_policy(KEY),
            crate::sync::policy::SettingPolicy::Local
        );
    }

    #[test]
    fn an_unreadable_saved_history_loads_as_empty() {
        let store = Store::open_in_memory().unwrap();
        store.set_setting(KEY, &"not a history").unwrap();
        assert!(History::load(&store).is_empty());
        store
            .set_setting(
                KEY,
                &serde_json::json!({"conversations": [{"scriptId": 5}]}),
            )
            .unwrap();
        assert!(History::load(&store).is_empty());
    }
}
