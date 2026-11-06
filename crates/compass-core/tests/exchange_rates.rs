//! Currency conversion: the ECB's daily file read, fed to fend, cached and
//! refreshed. Never the network: the ECB file is a checked-in fixture and
//! every fetch is a closure.

use std::collections::BTreeMap;
use std::sync::Arc;

use compass_core::calculator::{self, compute_with_rates};
use compass_core::exchange_rates::{
    ECB_DAILY_URL, ExchangeRates, MAX_AGE_SECS, ParseError, RateCache, cache_path, parse_ecb,
    source_url,
};

const FIXTURE: &str = include_str!("fixtures/eurofxref-daily.xml");

fn fixed_rates() -> Arc<ExchangeRates> {
    Arc::new(ExchangeRates {
        date: "2026-09-24".to_owned(),
        fetched_at: 0,
        rates: BTreeMap::from([
            ("EUR".to_owned(), 1.0),
            ("USD".to_owned(), 1.25),
            ("GBP".to_owned(), 0.8),
            ("JPY".to_owned(), 160.0),
        ]),
    })
}

fn answer(question: &str, rates: Option<Arc<ExchangeRates>>) -> Option<String> {
    compute_with_rates(question, rates).map(|answer| answer.answer)
}

#[test]
fn the_ecb_daily_file_is_read_with_its_date_and_the_euro() {
    let rates = parse_ecb(FIXTURE, 42).expect("the fixture parses");
    assert_eq!(rates.date, "2026-09-24");
    assert_eq!(rates.fetched_at, 42);
    assert_eq!(rates.rates.len(), 30, "29 currencies and the euro");
    assert_eq!(rates.rate("EUR"), Some(1.0));
    assert_eq!(rates.rate("USD"), Some(1.1367));
    assert_eq!(rates.rate("jpy"), Some(180.57), "any case");
    assert_eq!(rates.rate("IDR"), Some(20384.10));
    assert_eq!(rates.rate("XAU"), None);
}

#[test]
fn a_file_that_is_not_the_ecbs_is_refused_by_name() {
    assert!(matches!(parse_ecb("<nope", 0), Err(ParseError::Xml(_))));
    assert_eq!(parse_ecb("<Cube/>", 0), Err(ParseError::NoDate));
    assert_eq!(
        parse_ecb("<Cube><Cube time='2026-01-01'/></Cube>", 0),
        Err(ParseError::Empty)
    );
    assert_eq!(
        parse_ecb(
            "<Cube><Cube time='2026-01-01'><Cube currency='USD' rate='x'/></Cube></Cube>",
            0
        ),
        Err(ParseError::BadRate {
            currency: "USD".into(),
            rate: "x".into()
        })
    );
}

#[test]
fn currency_expressions_are_answered_with_the_rates() {
    let rates = Some(fixed_rates());
    assert_eq!(
        answer("10 usd to eur", rates.clone()).as_deref(),
        Some("8 EUR")
    );
    assert_eq!(
        answer("10 USD to EUR", rates.clone()).as_deref(),
        Some("8 EUR")
    );
    assert_eq!(answer("€5 in gbp", rates.clone()).as_deref(), Some("4 GBP"));
    assert_eq!(
        answer("2 * €2.5 to gbp", rates.clone()).as_deref(),
        Some("4 GBP")
    );
    assert_eq!(answer("$5 in eur", rates.clone()).as_deref(), Some("4 EUR"));
    assert_eq!(
        answer("£8 to usd", rates.clone()).as_deref(),
        Some("12.5 USD")
    );
    assert_eq!(
        answer("1 eur to jpy", rates.clone()).as_deref(),
        Some("160 JPY")
    );
    assert!(calculator::is_conversion("10 usd to eur"));
    assert!(calculator::is_conversion("€5 in gbp"));
}

#[test]
fn a_currency_the_rates_do_not_list_answers_nothing() {
    assert_eq!(answer("10 ars to eur", Some(fixed_rates())), None);
    assert_eq!(
        answer("2+2", Some(fixed_rates())).as_deref(),
        Some("4"),
        "arithmetic is untouched"
    );
}

#[test]
fn without_rates_currency_expressions_answer_nothing() {
    assert_eq!(answer("10 usd to eur", None), None);
    assert_eq!(answer("€5 in gbp", None), None);
    assert_eq!(answer("5 ft to m", None).as_deref(), Some("1.524 m"));
}

#[test]
fn only_an_expression_that_needs_a_rate_marks_the_rates_wanted() {
    let _ = calculator::take_rates_wanted();
    let _ = answer("5 ft to m", None);
    let _ = answer("2 + 2", None);
    assert!(
        !calculator::take_rates_wanted(),
        "units and arithmetic need none"
    );

    let _ = answer("10 usd to eur", None);
    assert!(calculator::take_rates_wanted(), "asked with no rates held");
    assert!(!calculator::take_rates_wanted(), "taking clears the mark");

    let _ = answer("10 usd to eur", Some(fixed_rates()));
    assert!(
        calculator::take_rates_wanted(),
        "asked with rates held too, so stale ones can be replaced"
    );
}

#[test]
fn the_fixture_s_rates_convert_through_the_euro() {
    let rates = Arc::new(parse_ecb(FIXTURE, 0).unwrap());
    let got = answer("100 eur to usd", Some(Arc::clone(&rates))).unwrap();
    assert_eq!(got, "113.67 USD");
    let got = answer("1136.7 usd to eur", Some(rates)).unwrap();
    assert_eq!(got, "1000 EUR");
}

#[test]
fn the_process_rates_reach_root_search() {
    calculator::set_exchange_rates(Some(fixed_rates()));
    let root = calculator::evaluate("10 usd to eur", false).map(|a| a.answer);
    let history = calculator::compute("10 usd to eur").map(|a| a.answer);
    calculator::set_exchange_rates(None);
    assert_eq!(root.as_deref(), Some("8 EUR"));
    assert_eq!(history.as_deref(), Some("8 EUR"));
}

#[test]
fn rates_go_stale_after_a_day_or_when_the_clock_goes_back() {
    let rates = ExchangeRates {
        fetched_at: 1_000,
        ..(*fixed_rates()).clone()
    };
    assert!(!rates.is_stale(1_000));
    assert!(!rates.is_stale(1_000 + MAX_AGE_SECS - 1));
    assert!(rates.is_stale(1_000 + MAX_AGE_SECS));
    assert!(rates.is_stale(999));
}

#[test]
fn the_source_is_the_ecb_unless_overridden() {
    assert_eq!(source_url(None), ECB_DAILY_URL);
    assert_eq!(source_url(Some("  ")), ECB_DAILY_URL);
    assert_eq!(
        source_url(Some("http://127.0.0.1:9/x.xml")),
        "http://127.0.0.1:9/x.xml"
    );
}

#[test]
fn a_refresh_keeps_and_caches_the_rates_and_a_restart_reads_them_back() {
    let home = tempfile::tempdir().unwrap();
    let path = cache_path(home.path());
    assert!(path.starts_with(home.path().join("compass")));

    let mut cache = RateCache::load(Some(path.clone()));
    assert!(cache.current().is_none());
    assert!(cache.needs_refresh(0), "no rates yet");

    let date = cache
        .refresh(|| Ok(FIXTURE.as_bytes().to_vec()), 5_000)
        .map(|rates| rates.date.clone())
        .unwrap();
    assert_eq!(date, "2026-09-24");
    assert!(!cache.needs_refresh(5_000 + 60));
    assert!(cache.needs_refresh(5_000 + MAX_AGE_SECS));

    let restarted = RateCache::load(Some(path));
    assert_eq!(restarted.current(), cache.current());
}

#[test]
fn a_failed_refresh_keeps_the_rates_in_hand() {
    let home = tempfile::tempdir().unwrap();
    let path = cache_path(home.path());
    let mut cache = RateCache::load(Some(path.clone()));
    cache
        .refresh(|| Ok(FIXTURE.as_bytes().to_vec()), 1)
        .unwrap();

    let offline = cache.refresh(|| Err("could not resolve the host".to_owned()), 2);
    assert_eq!(offline.unwrap_err(), "could not resolve the host");
    let garbled = cache.refresh(|| Ok(b"<html>maintenance</html>".to_vec()), 3);
    assert_eq!(garbled.unwrap_err(), ParseError::NoDate.to_string());

    assert_eq!(cache.current().map(|rates| rates.fetched_at), Some(1));
    assert_eq!(
        RateCache::load(Some(path)).current().map(|r| r.fetched_at),
        Some(1),
        "the cache file is untouched"
    );
}

#[test]
fn an_unreadable_cache_is_no_rates() {
    let home = tempfile::tempdir().unwrap();
    let path = cache_path(home.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{not json").unwrap();
    assert!(RateCache::load(Some(path)).current().is_none());
    assert!(RateCache::load(None).current().is_none());
}
