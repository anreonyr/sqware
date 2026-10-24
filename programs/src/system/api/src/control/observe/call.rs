use env::PieToken;

fn reply_to(request: &crate::control::observe::request::Request) -> PieToken {
    request.back
}

#[mold::contract(
    request = crate::control::observe::request::Request,
    response = crate::control::observe::reply::Reply,
    mark = crate::control::marks::OBSERVE_BACK,
    back = reply_to
)]
pub struct Call;
