use anyhow::Result;
use clap::Parser;
use krata::{events::EventStream, v1::control::control_service_client::ControlServiceClient};
use std::{
    io::{self, stdout, Stdout},
    time::Duration,
};
use tokio::select;
use tokio_stream::StreamExt;
use tonic::transport::Channel;

use crossterm::{
    event::{Event, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::*,
};
use ratatui::{
    prelude::*,
    symbols::border,
    widgets::{
        block::{Position, Title},
        Block, Borders, Row, Table, TableState,
    },
};

use crate::{
    format::zone_status_text,
    metrics::{
        lookup_metric_value, MultiMetricCollector, MultiMetricCollectorHandle, MultiMetricState,
    },
};

#[derive(Parser)]
#[command(about = "Dashboard for running zones")]
pub struct TopCommand {}

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

impl TopCommand {
    pub async fn run(
        self,
        client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        let collector = MultiMetricCollector::new(client, events, Duration::from_millis(200))?;
        let collector = collector.launch().await?;
        let mut tui = TopCommand::init()?;
        let mut app = TopApp {
            metrics: MultiMetricState { zones: vec![] },
            exit: false,
            table: TableState::new(),
        };
        app.run(collector, &mut tui).await?;
        TopCommand::restore()?;
        Ok(())
    }

    pub fn init() -> io::Result<Tui> {
        execute!(stdout(), EnterAlternateScreen)?;
        enable_raw_mode()?;
        Terminal::new(CrosstermBackend::new(stdout()))
    }

    pub fn restore() -> io::Result<()> {
        execute!(stdout(), LeaveAlternateScreen)?;
        disable_raw_mode()?;
        Ok(())
    }
}

pub struct TopApp {
    table: TableState,
    metrics: MultiMetricState,
    exit: bool,
}

impl TopApp {
    pub async fn run(
        &mut self,
        mut collector: MultiMetricCollectorHandle,
        terminal: &mut Tui,
    ) -> Result<()> {
        let mut events = crossterm::event::EventStream::new();

        while !self.exit {
            terminal.draw(|frame| self.render_frame(frame))?;

            select! {
                x = collector.receiver.recv() => match x {
                    Some(state) => {
                        self.metrics = state;
                    },

                    None => {
                        break;
                    }
                },

                x = events.next() => match x {
                    Some(event) => {
                        let event = event?;
                        self.handle_event(event)?;
                    },

                    None => {
                        break;
                    }
                }
            };
        }
        Ok(())
    }

    fn render_frame(&mut self, frame: &mut Frame) {
        frame.render_widget(self, frame.size());
    }

    fn handle_event(&mut self, event: Event) -> io::Result<()> {
        match event {
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                self.handle_key_event(key_event)
            }
            _ => {}
        };
        Ok(())
    }

    fn exit(&mut self) {
        self.exit = true;
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if let KeyCode::Char('q') = key_event.code {
            self.exit()
        }
    }
}

impl Widget for &mut TopApp {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = Title::from(" krata isolation engine ".bold());
        let instructions = Title::from(vec![" Quit ".into(), "<Q> ".blue().bold()]);
        let block = Block::default()
            .title(title.alignment(Alignment::Center))
            .title(
                instructions
                    .alignment(Alignment::Center)
                    .position(Position::Bottom),
            )
            .borders(Borders::ALL)
            .border_set(border::THICK);

        let mut rows = vec![];

        for ms in &self.metrics.zones {
            let Some(ref spec) = ms.zone.spec else {
                continue;
            };

            let Some(ref state) = ms.zone.state else {
                continue;
            };

            let memory_total = ms
                .root
                .as_ref()
                .and_then(|root| lookup_metric_value(root, "system/memory/total"));
            let memory_used = ms
                .root
                .as_ref()
                .and_then(|root| lookup_metric_value(root, "system/memory/used"));
            let memory_free = ms
                .root
                .as_ref()
                .and_then(|root| lookup_metric_value(root, "system/memory/free"));

            let row = Row::new(vec![
                spec.name.clone(),
                ms.zone.id.clone(),
                zone_status_text(state.status()),
                memory_total.unwrap_or_default(),
                memory_used.unwrap_or_default(),
                memory_free.unwrap_or_default(),
            ]);
            rows.push(row);
        }

        let widths = [
            Constraint::Min(8),
            Constraint::Min(8),
            Constraint::Min(8),
            Constraint::Min(8),
            Constraint::Min(8),
            Constraint::Min(8),
        ];

        let table = Table::new(rows, widths)
            .header(
                Row::new(vec![
                    "name",
                    "id",
                    "status",
                    "total memory",
                    "used memory",
                    "free memory",
                ])
                .style(Style::new().bold())
                .bottom_margin(1),
            )
            .column_spacing(1)
            .block(block);

        StatefulWidget::render(table, area, buf, &mut self.table);
    }
}
