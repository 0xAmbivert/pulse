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
        Self::new_with_mode(tick_rate, false)
    }

    pub fn new_with_mode(tick_rate: u64, headless: bool) -> Self {
        let (sender, receiver) = mpsc::channel(100);

        tokio::spawn(async move {
            let mut tick_interval = tokio::time::interval(Duration::from_millis(tick_rate));
            if headless {
                loop {
                    tick_interval.tick().await;
                    if sender.send(AppEvent::Tick).await.is_err() {
                        break;
                    }
                }
                return;
            }

            let mut reader = crossterm::event::EventStream::new();
            let mut input_active = true;

            loop {
                tokio::select! {
                    _ = tick_interval.tick() => {
                        if sender.send(AppEvent::Tick).await.is_err() {
                            break;
                        }
                    }
                    maybe_evt = reader.next(), if input_active => {
                        match maybe_evt {
                            Some(Ok(CrosstermEvent::Key(key))) => {
                                if sender.send(AppEvent::Input(key)).await.is_err() {
                                    break;
                                }
                            }
                            Some(Ok(_)) => {}
                            // Stream ended or TTY unavailable: stop polling reader to prevent a 100% CPU spin.
                            Some(Err(_)) | None => {
                                input_active = false;
                            }
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
