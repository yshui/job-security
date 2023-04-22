# job-security - job control from anywhere!

**job**-**s**ecurity is a tool that lets you put your running programs into background, then bring them to the foreground anywhere you want.

It also supplements shells that doesn't natively support job control, such as nushell, elvish, etc.

## Demo

![termtosvg_053kx7nv](https://user-images.githubusercontent.com/366851/233362873-b7ad80e9-8571-4236-a477-5b24b04f2261.svg)

## Features

- normal job control stuff: stopping things and putting them into the background, and resuming them later.
- job mobility: jobs are not tied to a terminal, you can resume stopped jobs wherever you want.
- starting/resuming jobs in the background.
- monitoring job statuses.
- preserving and retrieving logs from background jobs. output from background jobs won't invade your shell, and can be easily retrieved when needed.

## Installation

```bash
cargo install job-security
```

## Usage

- to run a command, use
  ```bash
  jobs run command -- arguments
  ```
- to suspend/stop a running program, Ctrl-Z!
- to resume, use
  ```bash
  jobs continue
  ```
- to list all jobs, use
  ```bash
  jobs list
  ```

## Limitations

- Terminal environment is generally not preserved. `jobs` tries to preserve the current working directory, and environment variables for the commands it spawns, but not much more. If you define aliases, functions, etc. in your shell, those will not be visible to the command you run.
- Not all shell expressions are supported. You can run zsh or bash expressions through `jobs`, as they will be automatically wrap in `zsh -c` or `bash -c`. But due to the limitations of other shells (e.g. nushell), commands are run as is, and not interpreted.
