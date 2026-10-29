use std::fmt;
use std::ops::{Add, AddAssign};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// The identifier a provider uses for a model, such as `openai/gpt-5-mini`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelId(String);

impl ModelId {
    /// Builds a model identifier, refusing an empty one.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::EmptyModelId`] when `id` is empty or blank.
    pub fn new(id: impl Into<String>) -> Result<Self, CoreError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(CoreError::EmptyModelId);
        }
        Ok(Self(id))
    }

    /// The identifier as the provider expects it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A number of tokens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TokenCount(pub u64);

/// What a call read from the provider's prompt cache, and wrote to it. A
/// provider that says nothing of its cache gives none of this: unknown, not
/// a cache that never hits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct CacheUse {
    /// Input tokens read from the cache.
    pub read: TokenCount,
    /// Input tokens written to it, when the provider says.
    pub written: Option<TokenCount>,
    /// What the call would have cost more had what it read been written to
    /// the cache again, as after an expiry, at the model's list prices,
    /// when they are known: what a warm cache saved it.
    #[serde(default)]
    pub rewrite_extra: Option<Usd>,
}

/// How long a provider keeps what is written to its prompt cache, for
/// the models whose cache is asked for by marks (Claude's). An hour costs
/// more to write, twice the input price against 1.25 times, but a pause
/// of up to an hour then needs no tick to keep it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheTtl {
    /// Five minutes, the providers' default.
    #[default]
    #[serde(rename = "5m")]
    FiveMinutes,
    /// An hour.
    #[serde(rename = "1h")]
    OneHour,
}

impl Add for TokenCount {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for TokenCount {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl fmt::Display for TokenCount {
    /// Prints counts the way a status line wants them: `842`, `8.2k`, `1.3M`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Integer arithmetic keeps the rounding exact; going through f64 would
        // print 8.2k for 8249 on one platform and 8.3k on another in theory,
        // and a status line is not the place to wonder about that.
        let n = self.0;
        if n < 1_000 {
            write!(f, "{n}")
        } else if n < 1_000_000 {
            write!(f, "{}.{}k", n / 1_000, (n % 1_000) / 100)
        } else {
            write!(f, "{}.{}M", n / 1_000_000, (n % 1_000_000) / 100_000)
        }
    }
}

/// An amount in US dollars.
///
/// Prices per token are fractions of a millionth of a dollar, which an integer
/// count of cents or micro-dollars would round to zero. Amounts are only ever
/// summed and displayed, never compared for equality, so `f64` is enough.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Usd(pub f64);

impl Add for Usd {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for Usd {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl fmt::Display for Usd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Cheap models cost fractions of a cent per request: below a cent four
        // decimals are shown, so that two requests can still be told apart.
        // Below what four decimals can show, saying so beats printing $0.0000,
        // which reads as free.
        // Zeros past the cents say nothing: $0.10, not $0.100.
        let text = match self.0 {
            0.0 => return f.write_str("$0.00"),
            x if x < 0.000_1 => return f.write_str("<$0.0001"),
            x if x < 0.01 => format!("{x:.4}"),
            x if x < 1.0 => format!("{x:.3}"),
            x => return write!(f, "${x:.2}"),
        };
        let (units, decimals) = text.split_once('.').unwrap_or((&text, ""));
        let decimals = decimals.trim_end_matches('0');
        write!(f, "${units}.{decimals:0<2}")
    }
}

/// A provider out of capacity for now: a quota spent for the minute, an
/// overload. The call is worth sending again, unlike one it refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Busy {
    /// How long it asked to be left before the next call, when it said.
    pub after: Option<Duration>,
}

/// What a model charges, per token, for what it reads and what it writes;
/// more, for some, once a call reads past a size of conversation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pricing {
    input_per_token: Usd,
    output_per_token: Usd,
    #[serde(default)]
    dearer: Option<Dearer>,
}

/// Where a model's price per token rises: from a call that reads that many
/// tokens, these prices.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Dearer {
    /// The tokens a call reads from which it is dearer.
    pub from: TokenCount,
    /// Dollars per token read, from there.
    pub input_per_token: Usd,
    /// Dollars per token written, from there.
    pub output_per_token: Usd,
}

impl Pricing {
    /// Builds a price list from per token amounts.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::InvalidPrice`] when either amount is negative,
    /// infinite or not a number.
    pub fn per_token(input: Usd, output: Usd) -> Result<Self, CoreError> {
        for price in [input, output] {
            if !price.0.is_finite() || price.0 < 0.0 {
                return Err(CoreError::InvalidPrice(price.0));
            }
        }
        Ok(Self {
            input_per_token: input,
            output_per_token: output,
            dearer: None,
        })
    }

    /// These prices, and `input` and `output` per token for a call that
    /// reads `from` tokens or more.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::InvalidPrice`] when either amount is negative,
    /// infinite or not a number.
    pub fn dearer_from(self, from: TokenCount, input: Usd, output: Usd) -> Result<Self, CoreError> {
        let dearer = Self::per_token(input, output)?;
        Ok(Self {
            dearer: Some(Dearer {
                from,
                input_per_token: dearer.input_per_token,
                output_per_token: dearer.output_per_token,
            }),
            ..self
        })
    }

    /// The tokens a call reads from which its price rises, when it does.
    #[must_use]
    pub fn rises_at(&self) -> Option<TokenCount> {
        self.dearer.map(|dearer| dearer.from)
    }

    /// What `usage` costs at these prices, the dearer ones for a call
    /// that read past where they start.
    ///
    /// # Examples
    ///
    /// ```
    /// use iq_core::{Pricing, TokenCount, Usage, Usd};
    ///
    /// let pricing = Pricing::per_token(Usd(1e-6), Usd(4e-6))?
    ///     .dearer_from(TokenCount(10_000), Usd(2e-6), Usd(6e-6))?;
    /// let usage = Usage { input: TokenCount(1_000), output: TokenCount(500) };
    /// assert!((pricing.cost(&usage).0 - 0.003).abs() < 1e-12);
    /// let long = Usage { input: TokenCount(10_000), output: TokenCount(500) };
    /// assert!((pricing.cost(&long).0 - 0.023).abs() < 1e-12);
    /// # Ok::<(), iq_core::CoreError>(())
    /// ```
    pub fn cost(&self, usage: &Usage) -> Usd {
        let (input, output) = match self.dearer {
            Some(dearer) if usage.input >= dearer.from => {
                (dearer.input_per_token, dearer.output_per_token)
            }
            _ => (self.input_per_token, self.output_per_token),
        };
        // Token counts stay far below 2^53, so the conversion is exact.
        Usd(usage.input.0 as f64 * input.0 + usage.output.0 as f64 * output.0)
    }
}

/// The tokens one request consumed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Tokens the model read: the prompt, the history, the context.
    pub input: TokenCount,
    /// Tokens the model wrote.
    pub output: TokenCount,
}

impl Add for Usage {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self {
            input: self.input + rhs.input,
            output: self.output + rhs.output,
        }
    }
}

impl AddAssign for Usage {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

/// A tool the model may ask to call, described the way models expect.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    /// The name the model uses to call it.
    pub name: String,
    /// When and why to call it. Models choose tools from this text.
    pub description: String,
    /// A JSON Schema object describing the arguments.
    pub parameters: serde_json::Value,
}

/// One call the model asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    /// The provider's identifier for this call, echoed back with the result.
    pub id: String,
    /// Which tool.
    pub name: String,
    /// The arguments, as the JSON text the model wrote. Kept as text because a
    /// model can write invalid JSON, and that is the tool's error to report
    /// back, not a transport failure.
    pub arguments: String,
}

/// One message in a conversation.
///
/// The variants carry exactly what each kind of message can hold, so that a
/// tool result without a call id, say, cannot be built.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    /// Standing instructions for the model.
    System(String),
    /// The person, or iq speaking on their behalf.
    User(String),
    /// What the model said, and the tools it asked to call.
    Assistant {
        /// Text, if the model wrote any alongside its calls.
        content: Option<String>,
        /// Calls to run before the conversation continues.
        tool_calls: Vec<ToolCall>,
    },
    /// The result of one tool call.
    Tool {
        /// The [`ToolCall::id`] this answers.
        call_id: String,
        /// What the tool returned, or why it failed.
        content: String,
    },
    /// An image a tool showed, sent after the turn's results: a tool result
    /// cannot carry one in the OpenAI format.
    Image {
        /// Its media type, such as `image/png`.
        mime: String,
        /// The image, in base64.
        data: String,
    },
}

impl Message {
    /// A message from the user.
    pub fn user(content: impl Into<String>) -> Self {
        Self::User(content.into())
    }

    /// A standing instruction for the model.
    pub fn system(content: impl Into<String>) -> Self {
        Self::System(content.into())
    }
}

/// A request for one answer from one model.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    /// The model that should answer.
    pub model: ModelId,
    /// The conversation so far, oldest first.
    pub messages: Vec<Message>,
    /// The tools the model may call. Empty means none.
    pub tools: Vec<ToolSpec>,
    /// How hard the model should think, for models that reason; `None`
    /// leaves it to the provider.
    pub effort: Option<Effort>,
}

/// How much a model reasons before it answers, in the five levels Requesty
/// understands.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    /// Quick answers.
    Low,
    /// Between the two.
    Medium,
    /// Careful work, the default.
    #[default]
    High,
    /// More than high, on the models that have it.
    Xhigh,
    /// As much as the model can.
    Max,
}

impl Effort {
    /// Every level, from the least to the most.
    pub const ALL: [Self; 5] = [Self::Low, Self::Medium, Self::High, Self::Xhigh, Self::Max];

    /// The level as the providers spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }

    /// The level `steps` away, staying within the levels.
    ///
    /// ```
    /// use iq_core::Effort;
    ///
    /// assert_eq!(Effort::High.step(1), Effort::Xhigh);
    /// assert_eq!(Effort::Low.step(-1), Effort::Low);
    /// ```
    pub fn step(self, steps: i32) -> Self {
        let at = Self::ALL.iter().position(|e| *e == self).unwrap_or(0) as i32;
        let last = Self::ALL.len() as i32 - 1;
        Self::ALL[(at + steps).clamp(0, last) as usize]
    }
}

impl fmt::Display for Effort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Effort {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|e| e.as_str() == s.trim().to_ascii_lowercase())
            .ok_or_else(|| {
                let levels: Vec<&str> = Self::ALL.iter().map(|e| e.as_str()).collect();
                format!("an effort is one of {}, got {s:?}", levels.join(", "))
            })
    }
}

/// A model's answer and what it cost.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatResponse {
    /// Text, if the model wrote any.
    pub content: Option<String>,
    /// Calls the model asked for. Empty means it considers itself done.
    pub tool_calls: Vec<ToolCall>,
    /// The tokens the request consumed, as the provider reported them.
    pub usage: Usage,
    /// The cost, when the provider reports it. Preferred over a price list
    /// lookup, since it accounts for caching and discounts the list cannot.
    pub cost: Option<Usd>,
    /// What it read from the prompt cache and wrote to it, when known.
    pub cache: Option<CacheUse>,
}

impl ChatResponse {
    /// The answer as a conversation message, to append before the next turn.
    pub fn to_message(&self) -> Message {
        Message::Assistant {
            content: self.content.clone(),
            tool_calls: self.tool_calls.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn model_id_refuses_blank() {
        assert_eq!(ModelId::new("  "), Err(CoreError::EmptyModelId));
        assert!(ModelId::new("openai/gpt-5-mini").is_ok());
    }

    #[test]
    fn token_count_display() {
        assert_eq!(TokenCount(842).to_string(), "842");
        assert_eq!(TokenCount(8_249).to_string(), "8.2k");
        assert_eq!(TokenCount(1_400).to_string(), "1.4k");
        assert_eq!(TokenCount(1_350_000).to_string(), "1.3M");
    }

    #[test]
    fn usd_display_never_rounds_a_cost_to_free() {
        assert_eq!(Usd(0.10).to_string(), "$0.10");
        assert_eq!(Usd(0.127).to_string(), "$0.127");
        assert_eq!(Usd(0.005).to_string(), "$0.005");
        assert_eq!(Usd(0.0).to_string(), "$0.00");
        assert_eq!(Usd(0.000_02).to_string(), "<$0.0001");
        assert_eq!(Usd(0.000_23).to_string(), "$0.0002");
        assert_eq!(Usd(0.041).to_string(), "$0.041");
        assert_eq!(Usd(12.345).to_string(), "$12.35");
    }

    #[test]
    fn pricing_refuses_nonsense() {
        assert!(Pricing::per_token(Usd(-1.0), Usd(0.0)).is_err());
        assert!(Pricing::per_token(Usd(0.0), Usd(f64::NAN)).is_err());
        assert!(Pricing::per_token(Usd(f64::INFINITY), Usd(0.0)).is_err());
    }

    proptest! {
        /// The session total must not depend on how it was split into requests.
        #[test]
        fn cost_is_additive(
            a_in in 0u64..10_000_000, a_out in 0u64..10_000_000,
            b_in in 0u64..10_000_000, b_out in 0u64..10_000_000,
            p_in in 0.0f64..1e-4, p_out in 0.0f64..1e-4,
        ) {
            let pricing = Pricing::per_token(Usd(p_in), Usd(p_out)).unwrap();
            let a = Usage { input: TokenCount(a_in), output: TokenCount(a_out) };
            let b = Usage { input: TokenCount(b_in), output: TokenCount(b_out) };
            let split = pricing.cost(&a).0 + pricing.cost(&b).0;
            let whole = pricing.cost(&(a + b)).0;
            prop_assert!((split - whole).abs() <= 1e-9 * whole.max(1.0));
        }
    }
}
