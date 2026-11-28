//! The inputs that name a point on the chart: `input.time` and `input.price`. A time and a price
//! that share an `inline` (in the same `group`, and no other time or price does) are one point,
//! set with one click, as TradingView does.

use crate::pine::compiler::program::{InputControl, InputDefinition, InputSchema, InputValue};
use crate::pine::inputs::values::{InputEdit, InputValues};
use crate::pine::syntax::token::ValueType;

/// Which coordinates a point sets, and the ids of the inputs that hold them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PointAxes {
    Time(String),
    Price(String),
    Both { time: String, price: String },
}

impl PointAxes {
    pub fn time(&self) -> Option<&str> {
        match self {
            Self::Time(time) | Self::Both { time, .. } => Some(time),
            Self::Price(_) => None,
        }
    }

    pub fn price(&self) -> Option<&str> {
        match self {
            Self::Price(price) | Self::Both { price, .. } => Some(price),
            Self::Time(_) => None,
        }
    }
}

/// Where a point is: the coordinates its inputs hold. A coordinate the point does not set is
/// `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointValue {
    pub time_ms: Option<i64>,
    pub price: Option<f64>,
}

/// One point of a script.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PointInput {
    pub axes: PointAxes,
    /// What the user is asked for: the `inline` of a pair, else the input's title.
    pub name: String,
    /// Whether adding the script asks for this point (`confirm` on it, or on either half).
    pub confirm: bool,
}

impl PointInput {
    /// Where the point is while the inputs hold `values` (an input with no entry holds its
    /// default).
    pub fn value(&self, schema: &InputSchema, values: &InputValues) -> PointValue {
        let held = |id: &str| {
            values.get(id).or_else(|| {
                schema
                    .inputs
                    .iter()
                    .find(|input| input.id == id)
                    .map(|input| &input.default_value)
            })
        };
        PointValue {
            time_ms: self.axes.time().and_then(|id| match held(id)? {
                InputValue::Int(time) => Some(*time),
                _ => None,
            }),
            price: self.axes.price().and_then(|id| match held(id)? {
                InputValue::Float(price) => Some(*price),
                _ => None,
            }),
        }
    }

    /// The edits that put the point at `time_ms` and `price`: one per coordinate it sets.
    pub fn moved_to(&self, time_ms: i64, price: f64) -> impl Iterator<Item = InputEdit> {
        let time = self
            .axes
            .time()
            .map(|id| InputEdit::Set(id.to_owned(), InputValue::Int(time_ms)));
        let price = self
            .axes
            .price()
            .map(|id| InputEdit::Set(id.to_owned(), InputValue::Float(price)));
        time.into_iter().chain(price)
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Axis {
    Time,
    Price,
}

fn axis(input: &InputDefinition) -> Option<Axis> {
    match (input.kind, input.control) {
        (ValueType::Time, _) => Some(Axis::Time),
        (_, InputControl::Price) => Some(Axis::Price),
        _ => None,
    }
}

/// The other half of `input`'s point: the one price that goes with a time, or the one time that
/// goes with a price. None unless `input` and it are the only time and price of their `inline`.
fn partner<'a>(schema: &'a InputSchema, input: &InputDefinition) -> Option<&'a InputDefinition> {
    let inline = input
        .inline
        .as_deref()
        .filter(|inline| !inline.is_empty())?;
    let sharing: Vec<(Axis, &InputDefinition)> = schema
        .inputs
        .iter()
        .filter(|other| other.inline.as_deref() == Some(inline) && other.group == input.group)
        .filter_map(|other| Some((axis(other)?, other)))
        .collect();
    match sharing.as_slice() {
        [(first_axis, first), (second_axis, second)] if first_axis != second_axis => {
            Some(if first.id == input.id { second } else { first })
        }
        _ => None,
    }
}

/// The script's points, in the order their first input is declared.
pub fn points(schema: &InputSchema) -> Vec<PointInput> {
    let mut paired: Vec<&str> = Vec::new();
    let mut points = Vec::new();
    for input in &schema.inputs {
        let Some(axis) = axis(input) else { continue };
        if paired.contains(&input.id.as_str()) {
            continue;
        }
        let title = || input.title.clone().unwrap_or_else(|| input.id.clone());
        points.push(match partner(schema, input) {
            Some(other) => {
                paired.push(&other.id);
                let (time, price) = match axis {
                    Axis::Time => (input, other),
                    Axis::Price => (other, input),
                };
                PointInput {
                    axes: PointAxes::Both {
                        time: time.id.clone(),
                        price: price.id.clone(),
                    },
                    name: input.inline.clone().unwrap_or_else(title),
                    confirm: input.confirm || other.confirm,
                }
            }
            None => PointInput {
                axes: match axis {
                    Axis::Time => PointAxes::Time(input.id.clone()),
                    Axis::Price => PointAxes::Price(input.id.clone()),
                },
                name: title(),
                confirm: input.confirm,
            },
        });
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::compiler::{self, NoLibraries};
    use crate::pine::syntax::token::Limits;

    fn of(body: &str) -> Vec<PointInput> {
        let source = format!("//@version=6\nindicator(\"T\")\n{body}\nplot(1)");
        let program = compiler::compile(&source, Limits::default(), &NoLibraries);
        assert!(program.is_valid(), "{:?}", program.diagnostics);
        points(&program.input_schema)
    }

    #[test]
    fn times_and_prices_are_points_in_declaration_order() {
        let found = of("len = input.int(14, \"Length\", confirm = true)\n\
             level = input.price(0, \"Stop level\", confirm = true)\n\
             until = input.time(0, \"End time\")");
        assert_eq!(
            found,
            [
                PointInput {
                    axes: PointAxes::Price("level".into()),
                    name: "Stop level".into(),
                    confirm: true,
                },
                PointInput {
                    axes: PointAxes::Time("until".into()),
                    name: "End time".into(),
                    confirm: false,
                },
            ]
        );
    }

    #[test]
    fn a_time_and_a_price_on_one_line_are_one_point_whatever_their_order() {
        let found = of(
            "p = input.price(0, \"Price\", inline = \"a\", confirm = true)\n\
             x = input.price(0, \"Between\")\n\
             t = input.time(0, \"Time\", inline = \"a\")",
        );
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[0],
            PointInput {
                axes: PointAxes::Both {
                    time: "t".into(),
                    price: "p".into(),
                },
                name: "a".into(),
                confirm: true,
            }
        );
        assert_eq!(found[1].axes, PointAxes::Price("x".into()));
    }

    #[test]
    fn another_group_or_a_third_input_on_the_line_leaves_them_apart() {
        let apart = of(
            "t = input.time(0, \"T\", inline = \"a\", group = \"one\")\n\
             p = input.price(0, \"P\", inline = \"a\", group = \"two\")",
        );
        assert_eq!(apart.len(), 2);
        let crowded = of("t = input.time(0, \"T\", inline = \"a\")\n\
             u = input.time(0, \"U\", inline = \"a\")\n\
             p = input.price(0, \"P\", inline = \"a\")");
        assert_eq!(crowded.len(), 3);
        let same_axis = of("t = input.time(0, \"T\", inline = \"a\")\n\
             u = input.time(0, \"U\", inline = \"a\")");
        assert_eq!(same_axis.len(), 2);
    }
}
