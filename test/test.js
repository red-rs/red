let start = Date.now()
let count = 0

for (let i = 0; i <= 100000000; i++) {
    count += i
}

let end = Date.now()
let elapsed = end - start
console.log(count, "elapsed", elapsed, "ms")