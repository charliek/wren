use std::future::Future;
use std::hash::Hash;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::advanced::subscription::{self, Event, EventStream, Recipe};
use iced::futures::{future, StreamExt};
use tokio::sync::oneshot;

type Waiter = (iced::window::Id, oneshot::Sender<()>);

#[derive(Clone, Default)]
pub struct Frames(Arc<Mutex<Vec<Waiter>>>);

impl Frames {
    pub fn subscription<Message: 'static + Send>(&self) -> iced::Subscription<Message> {
        subscription::from_recipe(Pulse::<Message> {
            frames: self.clone(),
            marker: std::marker::PhantomData,
        })
    }

    pub fn next(
        &self,
        window: iced::window::Id,
    ) -> impl Future<Output = Result<(), String>> + 'static {
        let (sender, receiver) = oneshot::channel();
        let mut waiters = self.0.lock().unwrap_or_else(|error| error.into_inner());
        waiters.retain(|(_, sender)| !sender.is_closed());
        waiters.push((window, sender));
        async move {
            // Software renderers on CI can take several seconds to present
            // the first frame of a large note; the harness bounds readiness.
            tokio::time::timeout(Duration::from_secs(60), receiver)
                .await
                .map_err(|_| "timed out waiting for a rendered frame".to_string())?
                .map_err(|_| "frame subscription ended".to_string())
        }
    }

    fn rendered(&self, window: iced::window::Id) {
        let mut waiters = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let mut index = 0;
        while index < waiters.len() {
            if waiters[index].0 == window || waiters[index].1.is_closed() {
                let (id, sender) = waiters.swap_remove(index);
                if id == window {
                    let _ = sender.send(());
                }
            } else {
                index += 1;
            }
        }
    }
}

struct Pulse<Message> {
    frames: Frames,
    marker: std::marker::PhantomData<Message>,
}

impl<Message: 'static + Send> Recipe for Pulse<Message> {
    type Output = Message;

    fn hash(&self, state: &mut subscription::Hasher) {
        std::any::TypeId::of::<Self>().hash(state);
        Arc::as_ptr(&self.frames.0).hash(state);
    }

    fn stream(
        self: Box<Self>,
        input: EventStream,
    ) -> iced::futures::stream::BoxStream<'static, Message> {
        input
            .filter_map(move |event| {
                if let Event::Interaction {
                    window,
                    event: iced::Event::Window(iced::window::Event::RedrawRequested(_)),
                    ..
                } = event
                {
                    self.frames.rendered(window);
                }
                future::ready(None)
            })
            .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_requested_window_releases_a_waiter() {
        let frames = Frames::default();
        let window = iced::window::Id::unique();
        let waiting = frames.next(window);
        frames.rendered(iced::window::Id::unique());
        assert_eq!(frames.0.lock().unwrap().len(), 1);
        frames.rendered(window);
        assert_eq!(frames.0.lock().unwrap().len(), 0);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        assert!(runtime.block_on(waiting).is_ok());
    }
}
