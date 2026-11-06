//! The calculator in root search.
//!
//! The C++ engine ships Numen, its own in-tree calculator; this uses
//! [`fend_core`], an existing Rust one with no dependencies of its own, which
//! covers arithmetic, percentages and unit conversions. Currency conversion
//! is fend's own, fed the ECB's daily rates ([`crate::exchange_rates`])
//! through its exchange-rate handler once [`set_exchange_rates`] has them;
//! without rates a currency expression answers nothing, as the C++ does when
//! its provider has none.
//!
//! When to try is the C++ rule (`RootSearchModel::refreshCalculator`): a query
//! starting with `=` always, otherwise only one of at least
//! [`MIN_CHARS`] characters that matched nothing else. One rule is added,
//! because fend reads almost any word as something (`a` is one ampere, `sin`
//! is the sine function): without the `=`, a query must contain a digit.

use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, Instant};

use crate::exchange_rates::ExchangeRates;

/// The shortest query tried without a leading `=`.
pub const MIN_CHARS: usize = 3;

/// How much work one evaluation may do before it is abandoned. A query is
/// typed a keystroke at a time; an answer later than this is worse than none.
///
/// It is the evaluating thread's CPU time, not the clock's: the limit is there
/// to stop an expression that would run away (`1000000!`), and a thread the
/// scheduler has not run yet has done no work. Against the clock, a busy
/// machine lost answers to `12*3+6`.
const TIME_LIMIT: Duration = Duration::from_millis(50);

/// The longest an evaluation may take by the clock, however little of it was
/// spent working: the bound on how long a starved evaluation holds a keystroke.
const WALL_LIMIT: Duration = Duration::from_secs(1);

/// A calculation and its result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The expression, as evaluated.
    pub question: String,
    /// The result, as fend prints it (`4`, `1.524 m`, `approx. 3.1415926536`).
    pub answer: String,
}

/// The rates currency expressions are answered with, process-wide: the
/// launcher asks its engine for them, and every evaluation reads them.
static RATES: RwLock<Option<Arc<ExchangeRates>>> = RwLock::new(None);

/// Answers currency expressions with `rates` from now on; `None` stops.
pub fn set_exchange_rates(rates: Option<Arc<ExchangeRates>>) {
    *RATES.write().unwrap_or_else(PoisonError::into_inner) = rates;
}

/// The rates currency expressions are answered with.
#[must_use]
pub fn exchange_rates() -> Option<Arc<ExchangeRates>> {
    RATES.read().unwrap_or_else(PoisonError::into_inner).clone()
}

thread_local! {
    /// Whether an evaluation on this thread has needed a rate since
    /// [`take_rates_wanted`] last looked. Per thread because evaluating is
    /// synchronous: the caller that evaluated is the one that asks.
    static RATES_WANTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether an evaluation on this thread needed an exchange rate since the
/// last call, held or not; clears the mark.
///
/// Rates are fetched on demand rather than kept fresh in the background, so
/// this is how a caller learns that a currency expression wants them: it
/// fetches when this is true and its rates are missing or stale, then
/// evaluates again.
#[must_use]
pub fn take_rates_wanted() -> bool {
    RATES_WANTED.with(|wanted| wanted.replace(false))
}

/// fend's exchange-rate handler over one day's rates, or over none yet: fend
/// asks for a currency's units per base currency, and the ECB's base is the
/// euro. Either way the ask is recorded for [`take_rates_wanted`].
struct RateHandler(Option<Arc<ExchangeRates>>);

impl fend_core::ExchangeRateFnV2 for RateHandler {
    fn relative_to_base_currency(
        &self,
        currency: &str,
        _options: &fend_core::ExchangeRateFnV2Options,
    ) -> Result<f64, Box<dyn std::error::Error + Send + Sync + 'static>> {
        RATES_WANTED.with(|wanted| wanted.set(true));
        let rates = self.0.as_ref().ok_or("no exchange rates yet")?;
        rates
            .rate(currency)
            .ok_or_else(|| format!("no exchange rate for {currency}").into())
    }
}

/// When an evaluation is abandoned: [`TIME_LIMIT`] of the thread's work, or
/// [`WALL_LIMIT`] by the clock, whichever comes first.
struct Deadline {
    work_started: Duration,
    wall_stop: Instant,
}

impl Deadline {
    fn start() -> Self {
        Self {
            work_started: thread_work(),
            wall_stop: Instant::now() + WALL_LIMIT,
        }
    }
}

impl fend_core::Interrupt for Deadline {
    fn should_interrupt(&self) -> bool {
        thread_work().saturating_sub(self.work_started) >= TIME_LIMIT
            || Instant::now() >= self.wall_stop
    }
}

/// The CPU time the calling thread has used.
#[cfg(unix)]
fn thread_work() -> Duration {
    use rustix::time::{ClockId, clock_gettime};
    let now = clock_gettime(ClockId::ThreadCPUTime);
    Duration::new(
        u64::try_from(now.tv_sec).unwrap_or(0),
        u32::try_from(now.tv_nsec).unwrap_or(0),
    )
}

/// Without a per-thread CPU clock the work is measured by the clock.
#[cfg(not(unix))]
fn thread_work() -> Duration {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed()
}

/// The answer to `query`, if it should be shown: `found_other` says whether
/// the root search found anything else.
#[must_use]
pub fn evaluate(query: &str, found_other: bool) -> Option<Answer> {
    let query = query.trim();
    let question = match query.strip_prefix('=') {
        Some(forced) => forced.trim(),
        None => {
            if found_other
                || query.chars().count() < MIN_CHARS
                || !query.chars().any(|c| c.is_ascii_digit())
            {
                return None;
            }
            query
        }
    };
    compute(question)
}

/// The answer to `question`, asked without the root list's gate: the
/// calculator history's live result, whose own gate is
/// [`crate::calculator_history::live_calc`].
#[must_use]
pub fn compute(question: &str) -> Option<Answer> {
    compute_with_rates(question, exchange_rates())
}

/// [`compute`] with these exchange rates rather than the process's.
#[must_use]
pub fn compute_with_rates(question: &str, rates: Option<Arc<ExchangeRates>>) -> Option<Answer> {
    let question = question.trim();
    if question.is_empty() {
        return None;
    }
    let mut context = fend_core::Context::new();
    context.set_exchange_rate_handler_v2(RateHandler(rates));
    let asked = euro_amounts_suffixed(question);
    let result =
        fend_core::evaluate_with_interrupt(&asked, &mut context, &Deadline::start()).ok()?;
    let answer = result.get_main_result().trim();
    // A lone number, or a function name, answers nothing.
    if result.output_is_empty() || answer.is_empty() || answer == question || answer == asked {
        return None;
    }
    Some(Answer {
        question: question.to_owned(),
        answer: answer.to_owned(),
    })
}

/// `question` with each euro amount written the way fend reads it: fend
/// takes `£8` and `$5` but reads `€5` as one unknown word, so a `€` right
/// before a number moves after it (`€5 in gbp` is asked as `5€ in gbp`).
fn euro_amounts_suffixed(question: &str) -> std::borrow::Cow<'_, str> {
    const EURO: char = '\u{20ac}';
    if !question.contains(EURO) {
        return std::borrow::Cow::Borrowed(question);
    }
    let mut out = String::with_capacity(question.len() + 2);
    let mut chars = question.chars().peekable();
    while let Some(c) = chars.next() {
        if c != EURO || !chars.peek().is_some_and(char::is_ascii_digit) {
            out.push(c);
            continue;
        }
        while let Some(&digit) = chars.peek() {
            if !(digit.is_ascii_digit() || matches!(digit, '.' | ',' | '_')) {
                break;
            }
            out.push(digit);
            chars.next();
        }
        out.push(EURO);
    }
    std::borrow::Cow::Owned(out)
}

/// Whether `question` converts between units (`5 ft to m`, `3 kg in lb`,
/// `100 C as F`, `10 usd to eur`) rather than computing: the C++ backends' `CONVERSION`
/// answer type, which fend does not report, read from its conversion
/// keywords.
#[must_use]
pub fn is_conversion(question: &str) -> bool {
    question.contains("->")
        || question
            .split_whitespace()
            .any(|word| matches!(word.to_lowercase().as_str(), "to" | "in" | "as"))
}

/// What copying `answer` puts on the clipboard: the value alone.
///
/// The C++ copies `answer.text`, and Numen keeps the unit apart from it
/// (`answer.unit`), so `100 usd to eur` copies `88.07` rather than the row's
/// `88.07 EUR`. fend prints one string, so the value is read back out of it:
/// its `approx. ` is dropped, and a unit after a lone number goes. An answer
/// that is not a number and a unit (`2 days, 3 hours`, a date, `true`) is
/// copied as shown.
#[must_use]
pub fn copied_value(answer: &str) -> String {
    let shown = answer.trim();
    let value = shown.strip_prefix("approx. ").unwrap_or(shown);
    match value.split_once(' ') {
        Some((number, unit))
            if number.parse::<f64>().is_ok() && !unit.chars().any(|c| c.is_ascii_digit()) =>
        {
            number.to_owned()
        }
        _ => value.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copying_an_answer_copies_its_value_without_approx_or_unit() {
        assert_eq!(copied_value("approx. 88.0669308675 EUR"), "88.0669308675");
        assert_eq!(copied_value("1.524 m"), "1.524");
        assert_eq!(copied_value("approx. 3.1415926536"), "3.1415926536");
        assert_eq!(copied_value("-40 °F"), "-40");
        assert_eq!(copied_value("4"), "4");
        assert_eq!(copied_value("2 days, 3 hours"), "2 days, 3 hours");
        assert_eq!(copied_value("true"), "true");
    }

    #[test]
    fn conversions_are_told_from_arithmetic_by_their_keyword() {
        assert!(is_conversion("5 ft to m"));
        assert!(is_conversion("3 kg IN lb"));
        assert!(is_conversion("100 °C -> °F"));
        assert!(is_conversion("0x10 as decimal"));
        assert!(!is_conversion("2+2*3"));
        assert!(!is_conversion("10% of 200"));
        assert!(
            !is_conversion("tonne + 1 kg"),
            "a word containing 'to' is not the keyword"
        );
    }

    #[test]
    fn compute_answers_without_the_root_lists_gate() {
        assert_eq!(
            compute("pi").map(|a| a.answer).as_deref(),
            Some("approx. 3.1415926536")
        );
        assert_eq!(compute(" 1+1 ").map(|a| a.question).as_deref(), Some("1+1"));
        assert_eq!(compute(""), None);
        assert_eq!(compute("firefox"), None);
    }

    fn answer(query: &str) -> Option<String> {
        evaluate(query, false).map(|a| a.answer)
    }

    #[test]
    fn arithmetic_percentages_and_units_are_answered() {
        assert_eq!(answer("2+2*3").as_deref(), Some("8"));
        assert_eq!(answer("10% of 200").as_deref(), Some("20"));
        assert_eq!(answer("5 ft to m").as_deref(), Some("1.524 m"));
        assert_eq!(answer("0.1+0.2").as_deref(), Some("0.3"));
    }

    #[test]
    fn words_short_queries_and_other_matches_are_left_alone() {
        assert_eq!(answer("firefox"), None);
        assert_eq!(answer("sin"), None, "a function name is not a question");
        assert_eq!(answer("1+"), None, "too short");
        assert_eq!(answer("12345"), None, "a number answers nothing");
        assert_eq!(evaluate("2+2+2", true), None, "something else matched");
        assert_eq!(answer("1/0"), None, "an error is no answer");
    }

    #[test]
    fn a_leading_equals_forces_it() {
        assert_eq!(
            evaluate("=pi", true).map(|a| a.answer).as_deref(),
            Some("approx. 3.1415926536")
        );
        assert_eq!(
            evaluate("= 1+1", true).map(|a| a.question).as_deref(),
            Some("1+1")
        );
        assert_eq!(evaluate("=", true), None);
    }

    #[test]
    fn a_slow_expression_is_abandoned_not_waited_for() {
        let started = Instant::now();
        let _ = evaluate("=2^2^2^2^2^2", false);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
