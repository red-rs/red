import time

start_time = time.time()
count = 0

for i in range(10):
    count += i
    print("hi", i)
    time.sleep(1)

print("done", count)
elapsed_time = time.time() - start_time
print("elapsed time:", elapsed_time, "seconds")
