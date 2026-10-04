use super::frame::Wire;
crate::table! {
    pub enum Grant {
        Build => "build", (Wire::Build(_) | Wire::Claim(_));
    }
    stem: "loader-entry-",
    wire_ty: Wire,
}
