//! What a run will cost, told before it starts: from the candidate's past
//! runs of the same exercise when there are some, else from a guess of
//! the tokens an agent spends on a small task, at the model's prices.

use iq_core::Usd;

use crate::candidate::Candidate;
use crate::record::Record;
use crate::roster::Roster;

/// What an agent reads on a small task, at a guess: its instructions, the
/// files it reads, its history read again at each call.
const GUESS_INPUT: u64 = 150_000;

/// What it writes.
const GUESS_OUTPUT: u64 = 12_000;

/// What a judge reads beside the candidates' work, and writes.
const JUDGE_INPUT: u64 = 4_000;
const JUDGE_OUTPUT: u64 = 1_500;

/// What one run of a candidate should cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Guess {
    /// The candidate.
    pub candidate: String,
    /// What the provider should bill, `None` when a price is unknown.
    pub billed: Option<Usd>,
    /// How many past runs it comes from: none for a guess.
    pub runs: usize,
}

/// What one run of `candidate` should cost: the mean of its `past` runs
/// when it has some, else the guess at the roster's prices.
#[must_use]
pub fn guess(candidate: &Candidate, past: &[Record], roster: &Roster) -> Guess {
    let name = candidate.to_string();
    let ran: Vec<&Record> = past
        .iter()
        .filter(|record| record.candidate.to_string() == name)
        .collect();
    if !ran.is_empty() {
        let n = ran.len() as f64;
        let billed = ran.iter().map(|r| r.spent.billed.0).sum::<f64>() / n;
        return Guess {
            candidate: name,
            billed: Some(Usd(billed)),
            runs: ran.len(),
        };
    }
    let mut billed = Some(Usd(0.0));
    for at in 0..candidate.agents() {
        let model = &candidate.models[at % candidate.models.len()];
        let priced = roster
            .offer(model.as_str())
            .and_then(|offer| Some((offer.input?, offer.output?)));
        billed = match (billed, priced) {
            (Some(sum), Some((input, output))) => Some(Usd(sum.0
                + GUESS_INPUT as f64 * input
                + GUESS_OUTPUT as f64 * output)),
            _ => None,
        };
    }
    Guess {
        candidate: name,
        billed,
        runs: 0,
    }
}

/// What judging `runs` runs should cost one judge, whose work to read is
/// `read` tokens in all: what it reads and writes, priced at `prices` per
/// input and output token; `None` when its prices are unknown.
#[must_use]
pub fn judging(read: u64, runs: usize, prices: Option<(f64, f64)>) -> Option<Usd> {
    let input = JUDGE_INPUT + read;
    let output = JUDGE_OUTPUT * runs.max(1) as u64 / 2 + JUDGE_OUTPUT;
    prices.map(|(i, o)| Usd(input as f64 * i + output as f64 * o))
}

/// `guesses` in a line, for the person to approve: the dollars billed, and
/// how much of it is a guess.
#[must_use]
pub fn said(guesses: &[Guess], times: usize) -> String {
    let times = times.max(1);
    let known = guesses.iter().filter(|g| g.runs > 0).count();
    let priced: Vec<f64> = guesses
        .iter()
        .filter_map(|g| g.billed)
        .map(|b| b.0)
        .collect();
    let unpriced = guesses.len() - priced.len();
    let dollars = Usd(priced.iter().sum::<f64>() * times as f64);
    let mut parts = Vec::new();
    if dollars.0 > 0.0 {
        parts.push(format!("about {dollars} billed"));
    }
    if unpriced > 0 {
        parts.push(format!("{unpriced} at a price the provider does not list"));
    }
    if parts.is_empty() {
        parts.push("nothing to speak of".into());
    }
    let basis = match known {
        0 => "guessed, as none of them ran it before".to_owned(),
        k if k == guesses.len() => "from their past runs".to_owned(),
        k => format!("{k} from past runs, the others guessed"),
    };
    format!("{} ({basis})", parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roster::Offer;

    #[test]
    fn a_guess_uses_past_runs_first_then_prices() {
        let roster = Roster {
            models: vec![Offer {
                id: "a/cheap".into(),
                canonical: None,
                input: Some(1e-6),
                output: Some(2e-6),
                context: None,
                tools: Some(true),
                description: None,
            }],
            ..Roster::default()
        };
        let cheap: Candidate = "a/cheap".parse().unwrap();
        let guessed = guess(&cheap, &[], &roster);
        assert!((guessed.billed.unwrap().0 - (0.15 + 0.024)).abs() < 1e-9);
        let pair: Candidate = "pair: a/cheap + a/cheap".parse().unwrap();
        let guessed = guess(&pair, &[], &roster);
        assert!((guessed.billed.unwrap().0 - 2.0 * (0.15 + 0.024)).abs() < 1e-9);
        let unknown: Candidate = "b/unlisted".parse().unwrap();
        assert_eq!(guess(&unknown, &[], &roster).billed, None);
        let text = said(
            &[guess(&cheap, &[], &roster), guess(&unknown, &[], &roster)],
            2,
        );
        assert!(text.starts_with("about $0.348 billed"), "{text}");
        assert!(text.contains("1 at a price") && text.contains("guessed"));
    }
}
