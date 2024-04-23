use std::thread;
use std::time::{Duration, Instant};

fn main() {
    let start_time = Instant::now();
    let mut count = 0;

    for i in 0..3 {
        count += i;
        println!("hi {}", i);
        thread::sleep(Duration::from_millis(1));
    }

    println!("done {}", count);
    let elapsed_time = start_time.elapsed();
    println!("elapsed time: {:?}", elapsed_time);
}