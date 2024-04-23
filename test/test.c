#include <stdio.h>

int main()
{
    int num1, num2, sum;

    // Prompt user to enter the first number
    printf("Enter the first number: ");
    scanf("%d", &num1);

    // Prompt user to enter the second number
    printf("Enter the second number: ");
    scanf("%d", &num2);

    // Calculate the sum
    sum = num1 + num2;

    // Display the result
    printf("The sum of %d and %d is: %d\n", num1, num2, sum);

    return 0;
}
