mod ast;

use inkwell::context::Context;
use inkwell::OptimizationLevel;
use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "syntax/main.pest"]
pub struct KomeParser;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() != 2 {
        println!("Usage: {} <source_file>", args[0]);
        return;
    }

    let source_file = args[1].clone();
    if let Err(e) = std::fs::read_to_string(&source_file) {
        println!("Error reading file {}: {}", source_file, e);
        return;
    }

    
}
