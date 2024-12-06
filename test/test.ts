class Person {
    name: string;
    age: number;

    constructor(name: string, age: number) {
        this.name = name;
        this.age = age;
    }

    greet(): string {
        return `Hello, my name is ${this.name} and I am ${this.age} years old.`;
    }
}

// Usage
const person = new Person("Alice", 25);
console.log(person.greet()); // Output: Hello, my name is Alice and I am 25 years old.
