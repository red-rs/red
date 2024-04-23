const std = @import("std");

pub fn fib(n: usize) -> u64 {
    var a: u64 = 0;
    var b: u64 = 1;
    var temp: u64;
    for (i := 0; i < n; i += 1) {
        temp = a + b;
        a = b;
        b = temp;
    }
    return a;
}

pub fn main() void {
    const n: usize = 10; // Change this value to generate Fibonacci sequence up to a different number.
    for (i := 0; i < n; i += 1) {
        const fib_number = fib(i);
        std.debug.print("{} ", .{fib_number});
    }
    std.debug.print("\n");
}
