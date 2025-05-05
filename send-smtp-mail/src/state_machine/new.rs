impl StateMachine {
    pub fn new(host: &str, port: u16) -> Self {
        StateMachine {
            state: State::Start,
            smtp_connection: stream::SmtpConnection::new(host, port),
        }
    }
}
