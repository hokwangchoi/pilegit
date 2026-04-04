pub mod app;
pub mod input;
pub mod ui;

use std::io;

use color_eyre::Result;
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::core::stack::Stack;
use crate::git::ops::Repo;
use app::App;

pub type Tui = Terminal<CrosstermBackend<io::Stdout>>;

/// Launch the interactive TUI.
pub fn run() -> Result<()> {
    let repo = Repo::open()?;
    let base = repo.detect_base()?;
    let commits = repo.list_stack_commits()?;
    let stack = Stack::new(base, commits);
    let mut app = App::new(stack);

    // Initialize terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Install panic hook to restore terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic);
    }));

    // Main loop
    let result = app.run(&mut terminal);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    result
}
