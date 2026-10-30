use super::*;
#[derive(Clone)]
pub struct Handle {
    pub(crate) tx: mpsc::Sender<Command>,
    pub(crate) state: watch::Receiver<Arc<State>>,
}
impl Handle {
    #[cfg(test)]
    pub fn testing() -> Self {
        Self {
            tx: mpsc::channel(256).0,
            state: watch::channel(Arc::new(State::default())).1,
        }
    }
    pub fn snapshot(&self) -> Arc<State> {
        self.state.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<Arc<State>> {
        self.state.clone()
    }
    pub fn send(&self, command: Command) -> Result<(), String> {
        self.tx
            .try_send(command)
            .map_err(|e| format!("Monitor queue unavailable: {e}"))
    }
    pub fn publish_from_worker(&self, command: Command) -> Result<(), String> {
        self.tx
            .blocking_send(command)
            .map_err(|_| "Monitor stopped".into())
    }
    pub async fn enqueue(&self, command: Command) -> Result<(), String> {
        self.tx
            .send(command)
            .await
            .map_err(|_| "Monitor stopped".into())
    }
    pub fn submit(&self, session: &Session, response: Response) -> Result<(), String> {
        self.send(Command::Submit {
            session: Box::new(session.clone()),
            response,
        })
    }
    pub fn open(&self, session: &Session) {
        let _ = self.send(Command::Open(session.key()));
    }
}
