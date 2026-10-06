//! OurStick's reading of the user's disks (ADR 0013): each disk's
//! judgement ([`share`]) and its turns ([`turns`]), for the catalogue, the
//! owner's files keeper and transfers alike.

pub mod share;
pub mod turns;
