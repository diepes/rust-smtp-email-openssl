// use send_smtp_mail::connect_to_server;
use send_smtp_mail::log4;
use send_smtp_mail::state_events;
use send_smtp_mail::state_machine;
use std::io;

// use local module smtp_starttls::smtp_starttls;
// mod smtp_starttls;

// https://learn.microsoft.com/en-us/azure/communication-services/concepts/service-limits

#[tokio::main]
async fn main() -> io::Result<()> {
    log4::init_log();

    // Do as little as possible in main.rs as it can't contain any tests
    let mut event_counter = 0;
    let mut state_machine = state_machine::StateMachine::new_from_env();
    log::info!(
        "Setup SMTP connection to {}:{}",
        state_machine.smtp_connection.host,
        state_machine.smtp_connection.port
    );
    let mut current_state = state_machine.state.clone();
    while match (&state_machine.state, event_counter) {
        (state_machine::State::Start, _) => {
            log::info!("Connecting to SMTP server...");
            state_machine
                .handle_event(state_events::Event::Connect)
                .await;
            true
        }
        (state_machine::State::Finished, i32::MIN..=15) => false,
        (_, 16..=i32::MAX) => {
            log::error!("Event counter exceeded 10 iterations, exiting.");
            false
        }
        (state_machine::State::Failed, _) => {
            log::error!("State machine failed, exiting.");
            false
        }
        (_s, _i) => true,
    } {
        event_counter += 1;
        log::debug!(
            "Loop iteration: {} state:{:?}",
            event_counter,
            state_machine.state
        );
        // Check if the current stream is None
        let new_event = state_events::get_event(&mut state_machine.smtp_connection).await;
        log::info!("Current event: {:?}", new_event);
        state_machine.handle_event(new_event).await;
        if current_state != state_machine.state {
            log::info!(
                "State changed from {:?} to {:?}",
                current_state,
                state_machine.state
            );
            current_state = state_machine.state.clone();
        }
    }
    // done
    let final_event = state_events::get_event(&mut state_machine.smtp_connection).await;
    log::info!("final event: {:?}", final_event);
    // from lib.rs call connect_to_server
    log::info!("SMTP Done server");

    //smtp_starttls::smtp_starttls(smtp_server, port).await
    Ok(())
}
