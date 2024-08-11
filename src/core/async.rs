use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::Duration;
use std::collections::BinaryHeap;
use std::cmp::Ordering;
use futures::task::noop_waker;

/// A task structure that contains a future and its priority.
struct Task {
    future: Pin<Box<dyn Future<Output = ()> + Send>>,
    priority: u8,
}

impl Task {
    /// Creates a new task with a specified priority.
    pub fn new(future: impl Future<Output = ()> + Send + 'static, priority: u8) -> Self {
        Self {
            future: Box::pin(future),
            priority,
        }
    }

    /// Polls the task using the provided context.
    pub fn poll(&mut self, context: &mut Context<'_>) -> Poll<()> {
        self.future.as_mut().poll(context)
    }
}

/// A priority-based executor that manages and runs tasks.
struct PriorityExecutor {
    tasks: BinaryHeap<ReverseTask>,
}

impl PriorityExecutor {
    /// Creates a new priority executor.
    pub fn new() -> Self {
        Self {
            tasks: BinaryHeap::new(),
        }
    }

    /// Spawns a new task into the executor with a specified priority.
    pub fn spawn(&mut self, future: impl Future<Output = ()> + Send + 'static, priority: u8) {
        self.tasks.push(ReverseTask(Task::new(future, priority)));
    }

    /// Runs all tasks managed by the executor in order of their priority.
    pub fn run(&mut self) {
        while let Some(mut task) = self.tasks.pop() {
            let waker = noop_waker();
            let mut context = Context::from_waker(&waker);
            match task.0.poll(&mut context) {
                Poll::Pending => {
                    self.tasks.push(task);
                }
                Poll::Ready(_) => {
                    // Task completed, do nothing
                }
            }
        }
    }
}

/// A wrapper around Task to implement reverse ordering for priority in BinaryHeap.
struct ReverseTask(Task);

impl Ord for ReverseTask {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.priority.cmp(&self.0.priority)
    }
}

impl PartialOrd for ReverseTask {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for ReverseTask {
    fn eq(&self, other: &Self) -> bool {
        self.0.priority == other.0.priority
    }
}

impl Eq for ReverseTask {}

/// Asynchronous sleep function that can be awaited.
pub async fn async_sleep(seconds: u64) {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(seconds));
        sender.send(()).expect("Failed to send completion signal");
    });
    receiver.await.expect("Failed to receive completion signal");
}

/// Executes a future synchronously, blocking the current thread until completion.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let waker = noop_waker();
        let mut context = Context::from_waker(&waker);
        let mut future = Box::pin(future);
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => {
                    tx.send(output).expect("Failed to send future result");
                    break;
                }
                Poll::Pending => thread::yield_now(),
            }
        }
    });
    rx.recv().expect("Failed to receive future result")
}

/// Example of a complex asynchronous task with priority.
async fn complex_task_example() {
    println!("Complex task started");
    async_sleep(2).await;
    println!("Complex task completed");
}

/// Example usage of the PriorityExecutor.
pub fn run_example() {
    let mut executor = PriorityExecutor::new();

    executor.spawn(complex_task_example(), 5);
    executor.spawn(async {
        println!("High priority task executed");
    }, 10);

    executor.spawn(async {
        println!("Low priority task executed");
    }, 1);

    executor.run();
}

fn main() {
    println!("Running async examples...");

    run_example();

    println!("Async examples completed.");
}

