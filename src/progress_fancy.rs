//! Build progress reporting for a "fancy" console, with progress bar etc.

use crate::progress::{build_message, Progress};
use crate::{
    graph::Build, graph::BuildId, process::Termination, task::TaskResult,
    work::BuildState, work::StateCounts, multivalue::MultiValueGauge
};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use std::io::Result;
use std::io::Write;

use crossterm;
use crossterm::execute;
use ratatui::Viewport;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Color;
use ratatui::widgets::{Block, Borders};
use ratatui::text::{Line, Text, Span};
use ratatui::{backend::CrosstermBackend, Terminal, TerminalOptions};
use ratatui_macros::span;

/// Currently running build task, as tracked for progress updates.
struct Task {
    id: BuildId,
    /// When the task started running.
    start: Instant,
    /// Build status message for the task.
    message: String,
    /// Last line of output from the task.
    last_line: Option<String>,
}

/// Progress implementation for "fancy" console, with progress bar etc.
/// Each time it prints, it clears from the cursor to the end of the console,
/// prints the status text, and then moves moves the cursor back up to the
/// start position.  This means on errors etc. we can clear any status by
/// clearing the console too.
pub struct FancyConsoleProgress {
    state: Arc<Mutex<FancyState>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Screen updates happen after this duration passes, to reduce the amount
/// of printing in the case of rapid updates.  This helps with terminal flicker.
const UPDATE_DELAY: Duration = std::time::Duration::from_millis(50);

/// If there are no updates for this duration, the progress will print anyway.
/// This lets the progress show ticking timers for long-running tasks so things
/// do not appear hung.
const TIMEOUT_DELAY: Duration = std::time::Duration::from_millis(500);

impl FancyConsoleProgress {
    pub fn new(verbose: bool) -> FancyConsoleProgress {
        let dirty_cond = Arc::new(Condvar::new());
        let state = Arc::new(Mutex::new(FancyState {
            done: false,
            pending: Vec::new(),
            dirty: false,
            dirty_cond: dirty_cond.clone(),
            counts: StateCounts::default(),
            tasks: VecDeque::new(),
            verbose,
        }));

	let height = 12;
	let mut stdout = std::io::stdout();
	
	let backend = CrosstermBackend::new(std::io::stdout());
	let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
		viewport: Viewport::Inline(height),
            },
	).unwrap();

        // Thread to debounce status updates -- waits a bit, then prints after
        // any dirty state.
        let thread = std::thread::spawn({
            let state_lock = state.clone();
            move || loop {
                // Wait to be notified of a display update or timeout.
                {
                    let (state, _) = dirty_cond
                        .wait_timeout_while(
                            state_lock.lock().unwrap(),
                            TIMEOUT_DELAY - UPDATE_DELAY,
                            |state| !state.done && !state.dirty,
                        )
                        .unwrap();
                    if state.done {
                        //std::io::stdout().write_all(&state.pending).unwrap();
			//crossterm::terminal::disable_raw_mode().unwrap();
			execute!(stdout,
				 crossterm::cursor::MoveDown(height)).unwrap();
			println!("\n");
                        break;
                    }
                }

                // Delay a little bit in case more display updates come in.
                // We know .dirty will only ever be cleared below, so we
                // can drop the lock here while we sleep.
                std::thread::sleep(UPDATE_DELAY);

                state_lock.lock().unwrap().print_progress(&mut terminal).unwrap();
            }
        });

        FancyConsoleProgress {
            state,
            thread: Some(thread),
        }
    }
}

impl Progress for FancyConsoleProgress {
    fn update(&self, counts: &StateCounts) {
        self.state.lock().unwrap().update(counts);
    }

    fn task_started(&self, id: BuildId, build: &Build) {
        self.state.lock().unwrap().task_started(id, build);
    }

    fn task_output(&self, id: BuildId, line: Vec<u8>) {
        self.state.lock().unwrap().task_output(id, line);
    }

    fn task_finished(&self, id: BuildId, build: &Build, result: &TaskResult) {
        self.state.lock().unwrap().task_finished(id, build, result);
    }

    fn log(&self, msg: &str) {
        self.state.lock().unwrap().log(msg);
    }
}

impl Drop for FancyConsoleProgress {
    fn drop(&mut self) {
        self.state.lock().unwrap().cleanup();
        self.thread.take().unwrap().join().unwrap();
    }
}

struct FancyState {
    done: bool,

    /// Text to print on the next update.
    /// Typically starts with the "clear any existing progress bar" sequence.
    pending: Vec<u8>,

    /// True when there is new progress to display.
    /// When set, will notify dirty_cond.
    dirty: bool,
    dirty_cond: Arc<Condvar>,

    /// Counts of tasks in each state.  TODO: pass this as function args?
    counts: StateCounts,
    /// Build tasks that are currently executing.
    /// Pushed to as tasks are started, so it's always in order of age.
    tasks: VecDeque<Task>,
    /// Whether to print command lines of started programs.
    verbose: bool,
}

impl FancyState {
    fn dirty(&mut self) {
        self.dirty = true;
        self.dirty_cond.notify_one();
    }

    fn update(&mut self, counts: &StateCounts) {
        self.counts = counts.clone();
        self.dirty();
    }

    fn task_started(&mut self, id: BuildId, build: &Build) {
        if self.verbose {
            write!(&mut self.pending, "{}\n", build.cmdline.as_ref().unwrap()).ok();
        }
        let message = build_message(build);
        self.tasks.push_back(Task {
            id,
            start: Instant::now(),
            message: message.to_string(),
            last_line: None,
        });
        self.dirty();
    }

    fn task_output(&mut self, id: BuildId, line: Vec<u8>) {
        let task = self.tasks.iter_mut().find(|t| t.id == id).unwrap();
        task.last_line = Some(String::from_utf8_lossy(&line).into_owned());
        self.dirty();
    }

    fn task_finished(&mut self, id: BuildId, build: &Build, result: &TaskResult) {
        self.tasks
            .remove(self.tasks.iter().position(|t| t.id == id).unwrap());

        // Show task name, status, and output.
        let buf = &mut self.pending;
        match result.termination {
            Termination::Success if result.output.is_empty() || build.hide_success => {
                // Common case: don't show anything.
                return;
            }
            Termination::Success => write!(buf, "{}\n", build_message(build)).ok(),
            Termination::Interrupted => write!(buf, "interrupted: {}\n", build_message(build)).ok(),
            Termination::Failure => write!(buf, "failed: {}\n", build_message(build)).ok(),
        };
        buf.extend_from_slice(&result.output);
        if !result.output.ends_with(b"\n") {
            buf.push(b'\n');
        }
        self.dirty();
    }

    fn log(&mut self, msg: &str) {
        self.pending.extend_from_slice(msg.as_bytes());
        self.pending.push(b'\n');
        self.dirty();
    }

    fn cleanup(&mut self) {
        self.done = true;
        self.dirty(); // let thread print final time
    }

    fn print_progress(&mut self, terminal:
		      &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
        let failed = self.counts.get(BuildState::Failed);
	let done = self.counts.get(BuildState::Done);
	let queued = self.counts.get(BuildState::Queued);
	let running = self.counts.get(BuildState::Running);
	let ready = self.counts.get(BuildState::Ready);
	let total = self.counts.total();
	let ratio = ((done as f64) / (total as f64))
	    .clamp(0.0, 1.0);
	let qratio = ((running as f64) / (total as f64))
	    .clamp(0.0, 1.0);
	let fratio = ((failed as f64) / (total as f64))
	    .clamp(0.0, 1.0);
	if ratio.is_nan() {
	    return Ok(());
	}
	let max_tasks = 8;
	let (cols, rows) = crossterm::terminal::size()?;
	let y = rows.saturating_sub(12);
	let area = Rect::new(0, y, cols, 12);
        let now = Instant::now();
	terminal.draw(|frame| {
            use Constraint::{Length, Fill};
	    let block = Block::default().title("N2").borders(Borders::ALL);
	    let inner = block.inner(area);
	    frame.render_widget(block, area);
            let layout = Layout::vertical([Length(1), Length(1), Length(1), Fill(8)]);
            let [line1, line2, _line3, line4] = inner.layout(&layout);
	    /*
	    let gauge = Gauge::default()
		.style(Modifier::BOLD)
		.gauge_style(Style::new().blue().on_black())
	    .ratio(ratio);

	    frame.render_widget(gauge, line1);
	    */
	    let mguage = MultiValueGauge::new().use_unicode(true)
		.add_value(ratio, Color::Blue)
		.add_value(qratio, Color::Green)
		.add_value(fratio, Color::Red);
	    frame.render_widget(mguage, line1);
	    let done_line = Line::from(
		format!("Done {} Queued {} Running {} Ready {} Failed {} Total {}",
			done, queued, running, ready, failed, total));
	    frame.render_widget(done_line, line2);
	    let mut messages: Vec<Line> = Vec::with_capacity(max_tasks);
            for task in self.tasks.iter().take(max_tasks) {
		let delta = now.duration_since(task.start).as_secs() as usize;
		let msg = task_message(&task.message, delta);
		messages.push(msg);
	    }
	    let tasks = Text::from(messages);
	    frame.render_widget(tasks, line4);
	})?;

        self.dirty = false;

	Ok(())
    }
}

/// Format a task's status message to optionally include how long it has been running
/// and also to fit within a maximum number of terminal columns.
fn task_message(message: &str, seconds: usize) -> Line<'_> {
    let mut spans : Vec<Span> = Vec::new();
    spans.push(span!(Color::Green; message));
    if seconds > 0 {
        spans.push(span!(format!(" ({}s)", seconds)))
    }
    Line::from(spans)
}
