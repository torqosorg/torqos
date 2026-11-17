//! The Agent Activity page (ADR-057): what the person's agent did, asked and read, from the model proxy's record,
//! as `show system agent activity` shows it — read by torqcli's own library (`torq::agent::activity`), the same
//! reading and the same words, never a second parser. By day: each conversation a row with Open, and the sites
//! asked for. Open: that conversation in order, as `show system agent activity conversation <id>` prints it.

use super::{blocking, failed, waiting, Loaded, Message, Settings};
use cosmic::iced::Length;
use cosmic::prelude::*;
use cosmic::widget::{self, settings};
use torq::agent::activity::{self, Days};

/// How many days the page lists, newest first; older ones are a terminal's (`show system agent activity day`).
const DAYS_SHOWN: usize = 14;
/// How much of one conversation the page draws, in bytes: the rest is a terminal's.
const CONVERSATION_SHOWN: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub enum Msg {
    /// The days of the record (None: no record yet), with the person they are for.
    Loaded(Result<(String, Option<Days>), String>),
    /// A conversation chosen, by its whole id.
    Open(String),
    /// That conversation, as the command line prints it.
    Opened(Result<String, String>),
    /// Back to the days.
    Back,
}

/// What the page holds.
#[derive(Default)]
pub(super) struct State {
    days: Option<Loaded<(String, Option<Days>)>>,
    /// The conversation open (its id), and what it says once read.
    open: Option<(String, Loaded<String>)>,
}

/// Ask for the days again (the conversation open, if one is, stays open).
pub(super) fn load() -> Task<cosmic::Action<Message>> {
    cosmic::task::future(async {
        Message::Activity(Msg::Loaded(
            blocking(|| {
                let me = torq::app::who()?;
                Ok((me.clone(), activity::days_in(&activity::folder(&me), None)?))
            })
            .await,
        ))
    })
}

/// One conversation, as `show system agent activity conversation <id>` prints it.
fn conversation(id: String) -> Task<cosmic::Action<Message>> {
    cosmic::task::future(async move {
        Message::Activity(Msg::Opened(
            blocking(move || {
                let me = torq::app::who()?;
                Ok(activity::shown(&me, &activity::View::Conversation(id))?)
            })
            .await,
        ))
    })
}

impl Settings {
    pub(super) fn activity_update(&mut self, m: Msg) -> Task<cosmic::Action<Message>> {
        let s = &mut self.activity;
        match m {
            Msg::Loaded(r) => s.days = Some(r.into()),
            Msg::Open(id) => {
                s.open = Some((id.clone(), Loaded::Waiting));
                return conversation(id);
            }
            Msg::Opened(r) => {
                if let Some((_, shown)) = s.open.as_mut() {
                    *shown = r.into();
                }
            }
            Msg::Back => s.open = None,
        }
        Task::none()
    }

    pub(super) fn activity_page(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let s = &self.activity;
        let mut page = widget::column::with_capacity(6).spacing(spacing.space_m);
        if let Some((id, shown)) = &s.open {
            page = page.push(
                widget::row::with_capacity(2)
                    .push(widget::button::standard("Back to the days").on_press(Message::Activity(Msg::Back)))
                    .push(widget::text::caption(format!("In a terminal: show system agent activity conversation {}", activity::short(id))))
                    .spacing(spacing.space_s)
                    .align_y(cosmic::iced::Alignment::Center),
            );
            return match shown {
                Loaded::Waiting => page.push(waiting()).into(),
                Loaded::Failed(why) => page.push(failed(why)).into(),
                Loaded::Got(text) => {
                    let cut = text.len() > CONVERSATION_SHOWN;
                    let mut end = text.len().min(CONVERSATION_SHOWN);
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    page = page.push(widget::text::monotext(text[..end].trim_end().to_string()).width(Length::Fill));
                    if cut {
                        page = page.push(widget::text::body("This is its beginning: the rest is longer than this page draws. In a terminal, the command above shows all of it."));
                    }
                    page.into()
                }
            };
        }
        page = page.push(widget::text::body(
            "What your agent asked its model, what it read (the output of each tool it used) and which sites it asked for, from the record TorqOS keeps of it. Only you can read your agent's record, and an administrator acting as root; your agent cannot read or change it. It holds what your agent read, so it is as private as your files. Times are in UTC.",
        ));
        let (me, days) = match &s.days {
            None | Some(Loaded::Waiting) => return page.push(waiting()).into(),
            Some(Loaded::Failed(why)) => return page.push(failed(why)).into(),
            Some(Loaded::Got(got)) => got,
        };
        let Some(days) = days else {
            return page.push(widget::text::body(activity::no_record(me).trim_end().to_string())).into();
        };
        if days.days.is_empty() {
            page = page.push(widget::text::body("Your agent's record holds nothing yet."));
        }
        for day in days.days.iter().take(DAYS_SHOWN) {
            let mut section = settings::section().title(day.day.clone());
            for c in &day.conversations {
                let open = c.id.clone().map(|id| widget::button::standard("Open").on_press(Message::Activity(Msg::Open(id))));
                let line = widget::text::body(activity::day_line(c)).width(Length::Fill);
                section = section.add(match open {
                    Some(b) => settings::item_row(vec![line.into(), b.into()]),
                    None => settings::item_row(vec![line.into()]),
                });
            }
            if !day.sites.is_empty() {
                let sites = day.sites.iter().map(activity::site_line).collect::<Vec<_>>().join("\n");
                section = section.add(settings::item_row(vec![widget::column::with_capacity(2).push(widget::text::body("Sites asked for")).push(widget::text::caption(sites)).width(Length::Fill).into()]));
            }
            for n in &day.notes {
                section = section.add(settings::item_row(vec![widget::text::caption(n.clone()).width(Length::Fill).into()]));
            }
            page = page.push(section);
        }
        if days.days.len() > DAYS_SHOWN {
            page = page.push(widget::text::caption(format!(
                "{} older days are kept. In a terminal, show system agent activity lists them all, and show system agent activity day <day> shows one.",
                days.days.len() - DAYS_SHOWN
            )));
        }
        let unread = days.unread.lines + days.unread.files.len() as u64;
        if unread > 0 {
            page = page.push(widget::text::caption("Some of the record could not be read: show system agent activity, in a terminal, says what."));
        }
        page.into()
    }
}
