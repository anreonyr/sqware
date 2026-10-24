use env::PieToken;
use super::{BACK, Page, Scan};

fn reply_to(frame: &Scan) -> PieToken { frame.back }

#[mold::contract(request = Scan, response = Page, mark = BACK, back = reply_to)]
pub struct Call;
