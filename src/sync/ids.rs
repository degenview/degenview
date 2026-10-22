//! Ids that mean the same thing on every Mac.
//!
//! Portfolios, paper-trading accounts and everything inside them are numbered 1, 2, 3… by each
//! Mac on its own, so two Macs both have a "portfolio 1" that are different things. Renumbering
//! local data would ripple through layouts, watchlist cards and open windows, so nothing local is
//! ever renumbered. Instead a small id is *tagged* with a number unique to this database when it
//! crosses into a synced record, and the tag is stripped again when this database's own record
//! comes back. The mapping is a pure function: no table, nothing to lose or to get out of step.
//!
//! * Below [`LEGACY_LIMIT`] an id is a per-Mac counter. It syncs as `GLOBAL_BASE | tag << 24 | id`.
//! * At or above it (nanosecond-timestamp ids from `new_id()`, and other Macs' tagged ids) an id is
//!   already unique and syncs unchanged.

use serde_json::Value;

/// Counter ids stay far below this: a Mac would need 16 million portfolios or orders.
pub const LEGACY_LIMIT: u64 = 1 << 24;
/// Tagged ids live in `[GLOBAL_BASE, GLOBAL_BASE << 1)`, above counters and below nanosecond ids.
pub const GLOBAL_BASE: u64 = 1 << 55;
const TAG_BITS: u32 = 31;
const TAG_MASK: u64 = (1 << TAG_BITS) - 1;

/// One database's id tag. Created with the database's sync identity and never changed, so a
/// database copied to a new Mac keeps meaning the same thing by the same ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdSpace {
    tag: u64,
}

impl IdSpace {
    pub fn new(tag: u32) -> Self {
        Self {
            tag: u64::from(tag) & TAG_MASK,
        }
    }

    /// The id as it appears in a synced record.
    pub fn to_global(self, local: u64) -> u64 {
        if local < LEGACY_LIMIT {
            GLOBAL_BASE | (self.tag << 24) | local
        } else {
            local
        }
    }

    /// The id as this Mac knows it. Another Mac's tagged ids are used as they are.
    pub fn to_local(self, global: u64) -> u64 {
        let tagged = (GLOBAL_BASE..GLOBAL_BASE << 1).contains(&global);
        if tagged && (global >> 24) & TAG_MASK == self.tag {
            global & (LEGACY_LIMIT - 1)
        } else {
            global
        }
    }

    pub fn to_global_string(self, local: &str) -> String {
        local
            .parse::<u64>()
            .map_or_else(|_| local.to_string(), |id| self.to_global(id).to_string())
    }

    pub fn to_local_string(self, global: &str) -> String {
        global
            .parse::<u64>()
            .map_or_else(|_| global.to_string(), |id| self.to_local(id).to_string())
    }
}

/// Which way an id is going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    ToGlobal,
    ToLocal,
}

impl IdSpace {
    pub fn map(self, direction: Direction, id: u64) -> u64 {
        match direction {
            Direction::ToGlobal => self.to_global(id),
            Direction::ToLocal => self.to_local(id),
        }
    }
}

/// Rewrites the number stored under each of `fields` in an object, if there is one.
pub fn map_fields(object: &mut Value, fields: &[&str], space: IdSpace, direction: Direction) {
    let Some(map) = object.as_object_mut() else {
        return;
    };
    for field in fields {
        if let Some(value) = map.get_mut(*field)
            && let Some(id) = value.as_u64()
        {
            *value = space.map(direction, id).into();
        }
    }
}

/// Rewrites the number stored under each of `fields` through `map`, for ids that are not
/// translated between spaces but replaced outright.
pub fn map_fields_with(object: &mut Value, fields: &[&str], map: &dyn Fn(u64) -> u64) {
    let Some(entries) = object.as_object_mut() else {
        return;
    };
    for field in fields {
        if let Some(value) = entries.get_mut(*field)
            && let Some(id) = value.as_u64()
        {
            *value = map(id).into();
        }
    }
}

/// Rewrites the portfolio id of a portfolio card, and the key derived from it, wherever a card
/// appears (a chart in a layout, a row of a watchlist). A card with no portfolio is every
/// portfolio and has nothing to rewrite.
pub fn map_card(card_owner: &mut Value, space: IdSpace, direction: Direction) {
    let Some(card) = card_owner.get_mut("card").filter(|card| card.is_object()) else {
        return;
    };
    let Some(old) = card.get("portfolio_id").and_then(Value::as_u64) else {
        return;
    };
    let new = space.map(direction, old);
    if new == old {
        return;
    }
    card["portfolio_id"] = new.into();
    // The card's identity string ends in the id: `portfolio:<kind>:<id>`.
    let (from, to) = (format!(":{old}"), format!(":{new}"));
    let rewrite = |text: &mut Value| {
        if let Some(key) = text.as_str()
            && key.starts_with("portfolio:")
            && let Some(prefix) = key.strip_suffix(&from)
        {
            *text = Value::String(format!("{prefix}{to}"));
        }
    };
    if let Some(symbol) = card_owner.get_mut("symbol") {
        rewrite(symbol);
    }
    if let Some(symbol) = card_owner.pointer_mut("/instrument/symbol") {
        rewrite(symbol);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counters_are_tagged_and_come_back_unchanged() {
        let a = IdSpace::new(0x1234_5678);
        for local in [1, 2, 99, LEGACY_LIMIT - 1] {
            let global = a.to_global(local);
            assert!((GLOBAL_BASE..GLOBAL_BASE << 1).contains(&global));
            assert_eq!(a.to_local(global), local);
        }
    }

    #[test]
    fn two_databases_never_share_a_global_id_for_the_same_counter() {
        let (a, b) = (IdSpace::new(1), IdSpace::new(2));
        for local in 1..200 {
            assert_ne!(a.to_global(local), b.to_global(local));
        }
        assert_eq!(a.to_global(7), a.to_global(7), "deterministic");
    }

    #[test]
    fn unique_ids_pass_through_both_ways() {
        let a = IdSpace::new(5);
        let nanos = 1_792_487_796_669_773_000_u64;
        assert_eq!(a.to_global(nanos), nanos);
        assert_eq!(a.to_local(nanos), nanos);
        assert_eq!(a.to_global(LEGACY_LIMIT), LEGACY_LIMIT);
    }

    #[test]
    fn another_databases_tagged_ids_are_used_as_they_are() {
        let (a, b) = (IdSpace::new(1), IdSpace::new(2));
        let theirs = b.to_global(3);
        assert_eq!(a.to_local(theirs), theirs, "never mistaken for one of mine");
        assert_eq!(b.to_local(theirs), 3);
    }

    #[test]
    fn nanosecond_ids_are_never_read_as_tagged() {
        let a = IdSpace::new(0x7fff_ffff);
        let nanos = 2_000_000_000_000_000_000_u64;
        assert_eq!(a.to_local(nanos), nanos);
    }

    #[test]
    fn string_ids_map_and_non_numeric_ids_are_left_alone() {
        let a = IdSpace::new(9);
        let global = a.to_global_string("4");
        assert_eq!(a.to_local_string(&global), "4");
        assert_eq!(a.to_global_string("favorites"), "favorites");
        assert_eq!(a.to_local_string("abc"), "abc");
    }

    #[test]
    fn fields_are_rewritten_in_place_and_missing_ones_ignored() {
        let a = IdSpace::new(3);
        let mut object = json!({"id": 5, "portfolio_id": 1, "note": "x", "other": null});
        map_fields(
            &mut object,
            &["id", "portfolio_id", "absent", "other"],
            a,
            Direction::ToGlobal,
        );
        assert_eq!(object["id"], a.to_global(5));
        assert_eq!(object["portfolio_id"], a.to_global(1));
        assert_eq!(object["note"], "x");
        map_fields(&mut object, &["id", "portfolio_id"], a, Direction::ToLocal);
        assert_eq!(object["id"], 5);
        assert_eq!(object["portfolio_id"], 1);
    }

    #[test]
    fn a_chart_card_and_its_key_follow_the_portfolio_id() {
        let a = IdSpace::new(3);
        let mut chart = json!({"id": 1, "symbol": "portfolio:value-chart:1",
            "card": {"kind": "ValueChart", "portfolio_id": 1, "range": "Month"}});
        map_card(&mut chart, a, Direction::ToGlobal);
        let global = a.to_global(1);
        assert_eq!(chart["card"]["portfolio_id"], global);
        assert_eq!(chart["symbol"], format!("portfolio:value-chart:{global}"));
        map_card(&mut chart, a, Direction::ToLocal);
        assert_eq!(chart["card"]["portfolio_id"], 1);
        assert_eq!(chart["symbol"], "portfolio:value-chart:1");
    }

    #[test]
    fn a_watchlist_card_row_follows_too_and_an_all_portfolios_card_is_untouched() {
        let a = IdSpace::new(3);
        let mut row = json!({"id": 9, "instrument": {"source": "coinmarketcap", "symbol": "portfolio:allocation:2"},
            "card": {"kind": "Allocation", "portfolio_id": 2}});
        map_card(&mut row, a, Direction::ToGlobal);
        assert_eq!(
            row["instrument"]["symbol"],
            format!("portfolio:allocation:{}", a.to_global(2))
        );
        let mut all = json!({"symbol": "portfolio:value:all", "card": {"kind": "Value", "portfolio_id": null}});
        let before = all.clone();
        map_card(&mut all, a, Direction::ToGlobal);
        assert_eq!(all, before);
        let mut plain = json!({"id": 1, "symbol": "BTCUSDT"});
        let before = plain.clone();
        map_card(&mut plain, a, Direction::ToGlobal);
        assert_eq!(plain, before);
    }
}
