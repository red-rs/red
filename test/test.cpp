#include <iostream>
#include <chrono>
#include <thread>

int main() {
    auto start_time = std::chrono::high_resolution_clock::now();
    int count = 0;

    for (int i = 0; i < 3; ++i) {
        count += i;
        std::cout << "hi " << i << std::endl;
        std::this_thread::sleep_for(std::chrono::milliseconds(1));
    }

    std::cout << "done " << count << std::endl;
    auto elapsed_time = std::chrono::high_resolution_clock::now() - start_time;
    auto elapsed_seconds = std::chrono::duration_cast<std::chrono::milliseconds>(elapsed_time);
    std::cout << "elapsed time: " << elapsed_seconds.count() / 1000.0 << " seconds" << std::endl;

    return 0;
}
