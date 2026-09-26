//! The one-shot `usagectl` command, a console program bundled beside the tray app.

fn main() {
    let arguments = std::env::args_os()
        .skip(1)
        .map(|argument| argument.to_string_lossy().into_owned());
    std::process::exit(uc_api::cli::main(arguments));
}
