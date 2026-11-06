//! The calculator's history: the C++ `CalculatorService`'s records, served
//! to Calculator History and fed by copying an answer (IPC v17).
//!
//! The rows live in the `calculator_history` table of Compass's own
//! encrypted database — the one extensions' local storage is in, keyed from
//! the login keyring, as the C++ keeps them in its encrypted `vicinae.db` —
//! so without a keyring the history is refused by name. Grouping by time is
//! `compass_local_storage::calculator::group_records_by_time` over the
//! boundaries this module reads from the local calendar, as the C++'s
//! `QDateTime::currentDateTime()`: the week starts on Monday
//! (`dayOfWeek() - 1`), and "today" ends a second before midnight.

use std::sync::Arc;

use compass_ipc::{
    CalculatorEdit, CalculatorGroup, CalculatorRecord, ErrorKind, ProtocolError, Request, Response,
};
use compass_local_storage::calculator::{
    AnswerType, History, TimeBoundaries, group_records_by_time, query_records,
};
use compass_sqlcipher_sys::rusqlite::Connection;
use tokio::sync::RwLock;

use super::EngineState;

const NO_KEYRING: &str = "calculator history needs the login keyring to open its database, \
                          and there is none on this session";

/// The group boundaries around `now`, in its time zone.
#[must_use]
pub fn boundaries(now: &jiff::Zoned) -> TimeBoundaries {
    let seconds = |zoned: Option<jiff::Zoned>| zoned.map_or(0, |z| z.timestamp().as_second());
    let day = now.date();
    let at_midnight = |date: jiff::civil::Date| date.to_zoned(now.time_zone().clone()).ok();
    let start_of_day = seconds(at_midnight(day));
    let week = day
        .checked_sub(jiff::Span::new().days(i64::from(day.weekday().to_monday_one_offset()) - 1))
        .unwrap_or(day);
    let month = day.first_of_month();
    let year = day.first_of_year();
    let next = |date: jiff::civil::Date, span: jiff::Span| {
        seconds(date.checked_add(span).ok().and_then(at_midnight))
    };
    TimeBoundaries {
        start_of_day,
        end_of_day: next(day, jiff::Span::new().days(1)) - 1,
        start_of_week: seconds(at_midnight(week)),
        end_of_week: next(week, jiff::Span::new().days(7)),
        start_of_month: seconds(at_midnight(month)),
        end_of_month: next(month, jiff::Span::new().months(1)),
        start_of_year: seconds(at_midnight(year)),
        end_of_year: next(year, jiff::Span::new().years(1)),
    }
}

/// Answers a calculator request against an open database, at `now`.
///
/// # Errors
///
/// None: a failed read or write is answered as an internal error.
pub fn answer(db: &Connection, request: Request, now: &jiff::Zoned, id: &str) -> Response {
    let history = History::new(db);
    let internal = |error: compass_local_storage::Error| {
        Response::Error(ProtocolError::new(ErrorKind::Internal, error.to_string()))
    };
    match request {
        Request::CalculatorHistory { query } => {
            let records = match history.list() {
                Ok(records) => records,
                Err(error) => return internal(error),
            };
            let groups = group_records_by_time(&query_records(&records, &query), boundaries(now));
            Response::CalculatorHistory {
                groups: groups
                    .into_iter()
                    .filter(|group| !group.records.is_empty())
                    .map(|group| CalculatorGroup {
                        name: group.name,
                        records: group
                            .records
                            .into_iter()
                            .map(|record| CalculatorRecord {
                                id: record.id,
                                question: record.question,
                                answer: record.answer,
                                conversion: record.answer_type == AnswerType::Conversion,
                                pinned: record.pinned_at.is_some(),
                            })
                            .collect(),
                    })
                    .collect(),
            }
        }
        Request::AddCalculatorRecord {
            question,
            answer,
            conversion,
        } => {
            let kind = if conversion {
                AnswerType::Conversion
            } else {
                AnswerType::Normal
            };
            match history.add(id, kind, &question, &answer, now.timestamp().as_second()) {
                Ok(()) => Response::Ack,
                Err(error) => internal(error),
            }
        }
        Request::EditCalculatorHistory { edit } => {
            let done = match &edit {
                CalculatorEdit::Pin(id) => history.pin(id, now.timestamp().as_second()),
                CalculatorEdit::Unpin(id) => history.unpin(id),
                CalculatorEdit::Remove(id) => history.remove(id),
                CalculatorEdit::RemoveAll => history.clear(),
            };
            match done {
                Ok(()) => Response::Ack,
                Err(error) => internal(error),
            }
        }
        other => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            format!("not a calculator request: {other:?}"),
        )),
    }
}

/// Opens the history's database, keyed from the keyring the first time;
/// Inspect Local Storage and Manage OAuth Token Sets read the same one.
pub(super) async fn storage(
    state: &Arc<RwLock<EngineState>>,
) -> Option<crate::extension_runner::Storage> {
    let slot = Arc::clone(&state.read().await.utilities.calculator);
    let mut slot = slot.lock().await;
    if slot.is_none() {
        let data_dir = compass_core::xdg_dirs::data_home()?.join("compass");
        *slot = super::extension_storage(&data_dir).await;
    }
    slot.clone()
}

/// A calculator request.
pub async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    let Some(storage) = storage(state).await else {
        return Response::Error(ProtocolError::new(ErrorKind::Unsupported, NO_KEYRING));
    };
    tokio::task::spawn_blocking(move || {
        let Some(db) = crate::extension_runner::open_storage(&storage) else {
            return Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                "the calculator history's database would not open",
            ));
        };
        let id = uuid::Uuid::new_v4().to_string();
        answer(&db, request, &jiff::Zoned::now(), &id)
    })
    .await
    .unwrap_or_else(|error| {
        Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            format!("calculator history task failed: {error}"),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> jiff::Zoned {
        text.parse().expect("a zoned time")
    }

    fn open() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = compass_sqlcipher_sys::open(&dir.path().join("history.db"), &[]).unwrap();
        compass_db::vicinae::run(&db).unwrap();
        (dir, db)
    }

    #[test]
    fn the_boundaries_are_the_cpps_monday_weeks_and_calendar_months() {
        // A Thursday in a leap February, in a zone with an offset.
        let now = at("2024-02-29T15:30:00+01:00[Europe/Paris]");
        let b = boundaries(&now);
        let second = |text: &str| at(text).timestamp().as_second();
        assert_eq!(
            b.start_of_day,
            second("2024-02-29T00:00:00+01:00[Europe/Paris]")
        );
        assert_eq!(
            b.end_of_day,
            second("2024-03-01T00:00:00+01:00[Europe/Paris]") - 1
        );
        assert_eq!(
            b.start_of_week,
            second("2024-02-26T00:00:00+01:00[Europe/Paris]")
        );
        assert_eq!(
            b.end_of_week,
            second("2024-03-04T00:00:00+01:00[Europe/Paris]")
        );
        assert_eq!(
            b.start_of_month,
            second("2024-02-01T00:00:00+01:00[Europe/Paris]")
        );
        assert_eq!(
            b.end_of_month,
            second("2024-03-01T00:00:00+01:00[Europe/Paris]")
        );
        assert_eq!(
            b.start_of_year,
            second("2024-01-01T00:00:00+01:00[Europe/Paris]")
        );
        assert_eq!(
            b.end_of_year,
            second("2025-01-01T00:00:00+01:00[Europe/Paris]")
        );

        // A Monday starts its own week.
        let monday = boundaries(&at("2024-02-26T08:00:00+01:00[Europe/Paris]"));
        assert_eq!(monday.start_of_week, monday.start_of_day);
    }

    #[test]
    fn copied_answers_are_remembered_grouped_filtered_pinned_and_removed() {
        let (_dir, db) = open();
        let now = at("2024-02-29T15:30:00+00:00[UTC]");
        let earlier = at("2023-06-01T10:00:00+00:00[UTC]");
        let add =
            |question: &str, answer_text: &str, conversion: bool, when: &jiff::Zoned, id: &str| {
                let request = Request::AddCalculatorRecord {
                    question: question.into(),
                    answer: answer_text.into(),
                    conversion,
                };
                assert_eq!(answer(&db, request, when, id), Response::Ack);
            };
        add("2+2", "4", false, &now, "a");
        add(
            "5 ft to m",
            "1.524 m",
            true,
            &at("2024-02-29T15:31:00+00:00[UTC]"),
            "b",
        );
        add("10% of 200", "20", false, &earlier, "c");

        let list = |query: &str| match answer(
            &db,
            Request::CalculatorHistory {
                query: query.into(),
            },
            &now,
            "",
        ) {
            Response::CalculatorHistory { groups } => groups,
            other => panic!("{other:?}"),
        };
        let shape = |groups: Vec<CalculatorGroup>| {
            groups
                .into_iter()
                .map(|g| {
                    (
                        g.name,
                        g.records.into_iter().map(|r| r.id).collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            shape(list("")),
            [
                ("Today".to_owned(), vec!["b".to_owned(), "a".to_owned()]),
                ("A few years ago".to_owned(), vec!["c".to_owned()]),
            ],
            "empty groups are left out"
        );
        let groups = list("");
        assert!(groups[0].records[0].conversion);
        assert!(!groups[0].records[1].conversion);

        assert_eq!(
            shape(list("200")),
            [("A few years ago".to_owned(), vec!["c".to_owned()])]
        );

        let edit = |edit: CalculatorEdit| {
            assert_eq!(
                answer(&db, Request::EditCalculatorHistory { edit }, &now, ""),
                Response::Ack
            );
        };
        edit(CalculatorEdit::Pin("c".into()));
        let groups = list("");
        assert_eq!(groups[0].name, "Pinned");
        assert!(groups[0].records[0].pinned);
        edit(CalculatorEdit::Unpin("c".into()));
        edit(CalculatorEdit::Remove("a".into()));
        assert_eq!(
            shape(list("")),
            [
                ("Today".to_owned(), vec!["b".to_owned()]),
                ("A few years ago".to_owned(), vec!["c".to_owned()]),
            ]
        );
        edit(CalculatorEdit::RemoveAll);
        assert!(list("").is_empty());
    }
}
