//! A link or bare ticket pasted by anyone: never a panic, and a ticket read
//! is written back as a link that reads as the same ticket.
#![no_main]

use std::str::FromStr;

use libfuzzer_sys::fuzz_target;
use ourstick_core::Ticket;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(ticket) = Ticket::from_str(text) else {
        return;
    };
    let link = ticket.to_link().expect("a ticket read can be written");
    let again = Ticket::from_str(&link).expect("a written link reads again");
    assert!(again == ticket, "a link does not read back as its ticket");
});
