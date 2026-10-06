//! The order of turns (ADR 0013 rule 5): in the order asked, urgent ones
//! first, at most three windows to one.

use super::*;

/// `ticket` takes its turn, reads a window and gives the turn back.
fn serve(state: &mut DiskState, ticket: u64) {
    state.grant(ticket);
    state.window_read();
    state.held = false;
}

#[test]
fn turns_go_in_order_and_urgent_ones_first_at_most_three_to_one() {
    let mut state = DiskState::default();
    let ask = |state: &mut DiskState, urgent: bool| {
        let ticket = state.next_ticket;
        state.next_ticket += 1;
        state.waiting.push_back((ticket, urgent));
        ticket
    };
    let catalogue = ask(&mut state, false);
    let other_stick = ask(&mut state, false);
    assert_eq!(state.chosen(), Some(catalogue));
    let urgent = ask(&mut state, true);
    // Urgent first…
    let mut granted = Vec::new();
    for _ in 0..3 {
        let chosen = state.chosen().unwrap();
        serve(&mut state, chosen);
        granted.push(chosen);
        // The urgent reader asks again at once.
        if chosen == urgent || granted.len() > 1 {
            ask(&mut state, true);
        }
    }
    assert_eq!(granted[0], urgent);
    // …at most three in a row while others wait, then the oldest other.
    let next = state.chosen().unwrap();
    serve(&mut state, next);
    assert_eq!(next, catalogue);
    assert_eq!(
        state.chosen().map(|ticket| ticket > other_stick),
        Some(true)
    );
    let urgent_again = state.chosen().unwrap();
    serve(&mut state, urgent_again);
    assert_ne!(urgent_again, other_stick);
}

#[test]
fn urgent_turns_and_others_alternate_three_to_one_for_good() {
    let mut state = DiskState::default();
    let ask = |state: &mut DiskState, urgent: bool| {
        let ticket = state.next_ticket;
        state.next_ticket += 1;
        state.waiting.push_back((ticket, urgent));
        ticket
    };
    // An urgent reader and a catalogue, each asking again as soon as served.
    let mut urgent_ticket = ask(&mut state, true);
    let mut other_ticket = ask(&mut state, false);
    let mut served = String::new();
    for _ in 0..16 {
        let chosen = state.chosen().unwrap();
        serve(&mut state, chosen);
        if chosen == urgent_ticket {
            served.push('u');
            urgent_ticket = ask(&mut state, true);
        } else {
            assert_eq!(chosen, other_ticket);
            served.push('o');
            other_ticket = ask(&mut state, false);
        }
    }
    assert_eq!(served, "uuuouuuouuuouuuo");
}

#[test]
fn urgent_windows_are_counted_not_urgent_turns() {
    let mut state = DiskState::default();
    let ask = |state: &mut DiskState, urgent: bool| {
        let ticket = state.next_ticket;
        state.next_ticket += 1;
        state.waiting.push_back((ticket, urgent));
        ticket
    };
    let other = ask(&mut state, false);
    // Transfers read one window in many short turns: still urgent's turn.
    for _ in 0..10 {
        let urgent = ask(&mut state, true);
        assert_eq!(state.chosen(), Some(urgent));
        state.grant(urgent);
        state.held = false;
    }
    // Three windows read: then the other's turn.
    for _ in 0..3 {
        let urgent = ask(&mut state, true);
        serve(&mut state, urgent);
    }
    ask(&mut state, true);
    assert_eq!(state.chosen(), Some(other));
}
