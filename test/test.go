package main

import (
	"fmt"
	"time"
)

func main() {
	start := time.Now()
	var count = 0
	for i := 0; i <= 100000000; i++ {
		count += i
	}

	fmt.Println(count, "elapsed", time.Since(start))
}
