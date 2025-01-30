# Contributing to Supercluster

Thank you for your interest in contributing to the Supercluster project!

We welcome contributions from the community and appreciate your efforts to improve the project.

## How to Contribute

There are several ways you can contribute to the project:

### Reporting Bugs

If you encounter any bugs, please [submit an issue](https://github.com/chargetrip/supercluster-rs/issues) with detailed information about the problem and steps to reproduce it. Include any relevant logs or screenshots that can help us understand the issue.

### Feature Requests

If you have ideas for new features, feel free to [submit an issue](https://github.com/chargetrip/supercluster-rs/issues) with a detailed description of the feature and its potential use cases. We appreciate your input and will consider your suggestions for future releases.

### Code of Conduct

Please note that this project is released with a Contributor Code of Conduct. By participating in this project, you agree to abide by its terms.

### Submitting Pull Requests

We welcome pull requests for bug fixes, new features, and improvements. To submit a pull request, follow these steps:

1. **Fork the repository**: Click the "Fork" button at the top right corner of the repository page to create a copy of the repository in your GitHub account.

2. **Clone the repository**: Clone your forked repository to your local machine using the following command:

    ```sh
    git clone git@github.com:chargetrip/supercluster-rs.git
    ```

3. **Create a new branch**: Create a new branch for your changes using the following command:

    ```sh
    git checkout -b my-feature-branch
    ```

4. **Make your changes**: Make the necessary changes to the codebase. Ensure that your code follows the project's coding standards and includes appropriate tests.

5. **Commit your changes**: Commit your changes with a descriptive commit message using the following command:

    ```sh
    git commit -m "feat: description of my changes"
    ```

6. **Push your changes**: Push your changes to your forked repository using the following command:

    ```sh
    git push origin my-feature-branch
    ```

7. **Create a pull request**: Go to the original repository and click the `New pull request` button. Select your branch and provide a detailed description of your changes. Submit the pull request for review.

### Building the project

To build the project, run the following command:

```sh
cargo build
```

### Testing the project

To run the tests, use:

```sh
cargo test
```

### Code Quality Checks

Run [clippy](https://github.com/rust-lang/rust-clippy) to lint the code:

```sh
cargo clippy --all-targets --all-features --no-deps -- -D warnings
```

Run [rustfmt](https://github.com/rust-lang/rustfmt) to format the code:

```sh
cargo fmt
```

### Documentation

Generate documentation in HTML format:

```bash
cargo doc --open
```


## Examples

We encourage contributors to create examples that demonstrate the features of the `supercluster` crate. Examples help users understand how to use the crate and provide practical use cases.

### Creating an Example

1. **Create a new example file**: Add a new file in the examples directory with a descriptive name, such as `examples/my_feature_example.rs`.

2. **Write the example code**: Write the code that demonstrates the feature. Ensure that the code is well-documented and includes comments explaining each step.

3. **Test the example**: Run the example to ensure it works as expected using the following command:

    ```sh
    cargo run --example my_feature_example
    ```

4. **Submit a pull request**: Follow the steps in the `Submitting Pull Requests` section to submit your example.

### Running Examples

To run an example, use the following command:

```sh
cargo run --example example_name
```

Replace `example_name` with the name of the example file (without the `.rs` extension).

## Getting Help

If you need help or have any questions, feel free to submit an issue or reach out to the maintainers.

Thank you for contributing to Supercluster!
