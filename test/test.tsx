import React from 'react';

interface GreetingProps {
    name: string;
}

const Greeting: React.FC<GreetingProps> = ({ name }) => {
    return (
        <div>
            <h1>Hello, {name}!</h1>
            <p>Welcome to the TypeScript and React world.</p>
        </div>
    );
};

export default Greeting;
