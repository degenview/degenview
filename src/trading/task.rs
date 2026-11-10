//! A request to a venue that has not answered yet, polled from the frame loop.
//!
//! Venues answer on their own schedule (a broker over the network, paper at once). The screens ask
//! and look again next frame; they never wait. A task keeps its answer once it has one.

pub struct Task<T> {
    poll: Box<dyn FnMut() -> Option<T> + Send>,
    done: Option<T>,
}

impl<T: Clone> Task<T> {
    /// A task that is answered by calling `poll` until it returns something.
    pub fn new(poll: impl FnMut() -> Option<T> + Send + 'static) -> Self {
        Self {
            poll: Box::new(poll),
            done: None,
        }
    }

    /// A task that is already answered.
    pub fn ready(value: T) -> Self {
        Self {
            poll: Box::new(|| None),
            done: Some(value),
        }
    }

    /// The answer, once there is one; the same answer every time after.
    pub fn poll(&mut self) -> Option<&T> {
        if self.done.is_none() {
            self.done = (self.poll)();
        }
        self.done.as_ref()
    }

    pub fn is_done(&self) -> bool {
        self.done.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_keeps_its_answer() {
        let mut calls = 0;
        let mut task = Task::new(move || {
            calls += 1;
            (calls >= 3).then_some(calls)
        });
        assert_eq!(task.poll(), None);
        assert_eq!(task.poll(), None);
        assert_eq!(task.poll(), Some(&3));
        assert_eq!(task.poll(), Some(&3), "asked once more, the same answer");
        assert!(task.is_done());
        assert_eq!(Task::ready(7).poll(), Some(&7));
    }
}
