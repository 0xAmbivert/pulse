use crossterm::event::{Event as CrosstermEvent, KeyEvent};
use futures::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;

pub enum AppEvent {
    Input(KeyEvent),
    Tick,
}

pub struct EventHandler {
    receiver: mpsc::Receiver<AppEvent>,
}

impl EventHandler {
    pub fn new(tick_rate: u64) -> Self {
        let (sender, receiver) = mpsc::channel(100);

        tokio::spawn(async move {
            let mut reader = crossterm::event::EventStream::new();
            let mut tick_interval = tokio::time::interval(Duration::from_millis(tick_rate));

            loop {
                tokio::select! {
                    _ = tick_interval.tick() => {
                        if sender.send(AppEvent::Tick).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(evt)) = reader.next() => {
                        match evt {
                            CrosstermEvent::Key(key) => {
                                if sender.send(AppEvent::Input(key)).await.is_err() {
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        });

        Self { receiver }
    }

    pub async fn next(&mut self) -> Option<AppEvent> {
        self.receiver.recv().await
    }
}
