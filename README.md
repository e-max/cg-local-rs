# cg-local-rs

A background process for syncing [Codingame IDE](https://www.codingame.com/) with a local file. 
It allows using your preferred IDE for solving puzzles. 

This app is compatible with browser extentions 
* [Firefox](https://addons.mozilla.org/en-US/firefox/addon/cg-local/)
* [Chrome](https://chrome.google.com/webstore/detail/cg-local/ihakjfajoihlncbnggmcmmeabclpfdgo)

**cg-local-rs** is a replacement for a  [CG Local App](https://github.com/jmerle/cg-local-app) which is required an old version of Java
and the old version of openjfx and might be hard to install. 

Original thread about CG Local is here https://www.codingame.com/forum/t/cg-local/10359

## Installation

### Using cargo
```
$ cargo install cg-local-rs
```

### From the prebuilt release

Download the latest release 
https://github.com/e-max/cg-local-rs/releases



## Usage
First, you need to install an extension for your browser [Firefox](https://addons.mozilla.org/en-US/firefox/addon/cg-local/), [Chrome](https://chrome.google.com/webstore/detail/cg-local/ihakjfajoihlncbnggmcmmeabclpfdgo).


Then you need to choose a local file you want to synchronize with CodiGame and pass the name to **cg-local--rs**
```
$ cg-local-rs ./src/main.rs
```
That's all!  All changes you make in your file will be immediately uploaded to CodinGame IDE.

We consider your local file state superior to CodinGame IDE state and sync only in one direction - from your local file to CodinGame IDE.

There are two exceptions: 
* if the local file is empty when you run the tool, it will download state from CodinGame IDE.
* when you start the tool with a flag **--force-first-download**




