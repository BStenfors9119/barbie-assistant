mod app;
mod builder_state;
mod commands;
mod settings;
mod stm_schema;
mod templates;
mod travel_request;
mod user_templates;
mod utils;

fn main() -> iced::Result {
    iced::application(
        app::App::title,
        app::App::update,
        app::App::view,
    )
    .theme(app::App::theme)
    .run()
}
