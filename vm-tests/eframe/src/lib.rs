//! The only piece of egui the party client touches.

pub mod egui {
    #[derive(Clone, Debug, Default)]
    pub struct Context;

    impl Context {
        /// The real context schedules a repaint; here nothing draws.
        pub fn request_repaint(&self) {}
    }
}
