use kome_ast::expressions::{BinaryExpression, BinaryOp};
use kome_ast::{
    declarations::{Declaration, FunctionDeclaration, Module},
    expressions::{CallArg, CallExpression, Expression, LiteralKind},
    patterns::Pattern,
    statements::{BlockStatement, Statement},
};
use std::{collections::HashMap, fmt};

pub mod abi;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    String(String),
    Number(f64),
    Boolean(bool),
    Null,
}

impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(value) => formatter.write_str(value),

            Self::Number(value) => {
                write!(formatter, "{value}")
            }

            Self::Boolean(value) => {
                write!(formatter, "{value}")
            }

            Self::Null => formatter.write_str("null"),
        }
    }
}

#[derive(Debug)]
pub enum RuntimeError {
    DuplicateFunction {
        name: String,
    },

    FunctionNotFound {
        name: String,
    },

    MissingFunctionBody {
        name: String,
    },

    InvalidNativeAttribute {
        function: String,
        message: String,
    },

    NativeFunctionNotFound {
        name: String,
    },

    ArgumentCount {
        function: String,
        expected: usize,
        actual: usize,
    },

    UnsupportedParameterPattern {
        function: String,
    },

    UnsupportedExpression {
        kind: &'static str,
    },

    UnsupportedStatement {
        kind: &'static str,
    },

    UndefinedVariable {
        name: String,
    },

    InvalidNumber {
        value: String,
    },

    NoActiveScope,

    Native {
        message: String,
    },

    TypeError {
        message: String,
    },

    InvalidAssignmentTarget,

    ImmutableVariable {
        name: String,
    },
}

impl RuntimeError {
    pub fn native(message: impl Into<String>) -> Self {
        Self::Native {
            message: message.into(),
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateFunction { name } => {
                write!(formatter, "duplicate runtime function `{name}`",)
            }

            Self::FunctionNotFound { name } => {
                write!(formatter, "function `{name}` was not found",)
            }

            Self::MissingFunctionBody { name } => {
                write!(formatter, "function `{name}` has no body",)
            }

            Self::InvalidNativeAttribute { function, message } => {
                write!(
                    formatter,
                    "invalid @native attribute on `{function}`: {message}",
                )
            }

            Self::NativeFunctionNotFound { name } => {
                write!(formatter, "native function `{name}` is not registered",)
            }

            Self::ArgumentCount {
                function,
                expected,
                actual,
            } => {
                write!(
                    formatter,
                    "function `{function}` expects {expected} argument(s), \
                     but received {actual}",
                )
            }

            Self::UnsupportedParameterPattern { function } => {
                write!(
                    formatter,
                    "function `{function}` contains an unsupported parameter pattern",
                )
            }

            Self::UnsupportedExpression { kind } => {
                write!(formatter, "expression `{kind}` is not supported yet",)
            }

            Self::UnsupportedStatement { kind } => {
                write!(formatter, "statement `{kind}` is not supported yet",)
            }

            Self::UndefinedVariable { name } => {
                write!(formatter, "variable `{name}` is not defined",)
            }

            Self::InvalidNumber { value } => {
                write!(formatter, "`{value}` is not a valid number",)
            }

            Self::NoActiveScope => formatter.write_str("internal error: no active runtime scope"),

            Self::Native { message } => {
                write!(formatter, "native function failed: {message}",)
            }

            Self::TypeError { message } => {
                write!(formatter, "type error: {message}")
            }

            Self::InvalidAssignmentTarget => formatter.write_str("invalid assignment target"),

            Self::ImmutableVariable { name } => {
                write!(formatter, "cannot assign to immutable variable `{name}`",)
            }
        }
    }
}

impl std::error::Error for RuntimeError {}

type NativeFunction = dyn Fn(&[Value]) -> Result<Value, RuntimeError> + Send + Sync + 'static;

#[derive(Default)]
pub struct NativeRegistry {
    functions: HashMap<String, Box<NativeFunction>>,
}

impl NativeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<F>(&mut self, name: impl Into<String>, function: F)
    where
        F: Fn(&[Value]) -> Result<Value, RuntimeError> + Send + Sync + 'static,
    {
        self.functions.insert(name.into(), Box::new(function));
    }

    pub fn call(&self, name: &str, arguments: &[Value]) -> Result<Value, RuntimeError> {
        let function =
            self.functions
                .get(name)
                .ok_or_else(|| RuntimeError::NativeFunctionNotFound {
                    name: name.to_string(),
                })?;

        function(arguments)
    }
}

pub struct Interpreter<'a> {
    functions: HashMap<&'a str, &'a FunctionDeclaration>,
    natives: &'a NativeRegistry,
    scopes: Vec<HashMap<String, Value>>,
}

impl<'a> Interpreter<'a> {
    pub fn new(module: &'a Module, natives: &'a NativeRegistry) -> Result<Self, RuntimeError> {
        let mut functions = HashMap::new();

        for declaration in &module.declarations {
            let Declaration::Function(function) = declaration else {
                continue;
            };

            if functions.insert(function.name.as_str(), function).is_some() {
                return Err(RuntimeError::DuplicateFunction {
                    name: function.name.clone(),
                });
            }
        }

        Ok(Self {
            functions,
            natives,
            scopes: Vec::new(),
        })
    }

    pub fn run_entry(&mut self, name: &str) -> Result<Value, RuntimeError> {
        if !self.functions.contains_key(name) {
            return Err(RuntimeError::FunctionNotFound {
                name: name.to_string(),
            });
        }

        self.call_function(name, Vec::new())
    }

    fn call_function(&mut self, name: &str, arguments: Vec<Value>) -> Result<Value, RuntimeError> {
        let function = *self
            .functions
            .get(name)
            .ok_or_else(|| RuntimeError::FunctionNotFound {
                name: name.to_string(),
            })?;

        if function.params.len() != arguments.len() {
            return Err(RuntimeError::ArgumentCount {
                function: name.to_string(),
                expected: function.params.len(),
                actual: arguments.len(),
            });
        }

        if let Some(native_name) = native_function_name(function)? {
            return self.natives.call(&native_name, &arguments);
        }

        let body = function
            .body
            .as_ref()
            .ok_or_else(|| RuntimeError::MissingFunctionBody {
                name: name.to_string(),
            })?;

        let mut function_scope = HashMap::new();

        for (parameter, argument) in function.params.iter().zip(arguments) {
            let Pattern::Ident(parameter) = parameter else {
                return Err(RuntimeError::UnsupportedParameterPattern {
                    function: name.to_string(),
                });
            };

            function_scope.insert(parameter.name.clone(), argument);
        }

        self.scopes.push(function_scope);

        let result = self.execute_block(body);

        self.scopes.pop();

        match result? {
            Flow::Next => Ok(Value::Null),
            Flow::Return(value) => Ok(value),
        }
    }

    fn execute_block(&mut self, block: &BlockStatement) -> Result<Flow, RuntimeError> {
        self.scopes.push(HashMap::new());

        let result = (|| {
            for statement in &block.statements {
                let flow = self.execute_statement(statement)?;

                if let Flow::Return(_) = flow {
                    return Ok(flow);
                }
            }

            Ok(Flow::Next)
        })();

        self.scopes.pop();

        result
    }

    fn execute_statement(&mut self, statement: &Statement) -> Result<Flow, RuntimeError> {
        match statement {
            Statement::Empty(_) => Ok(Flow::Next),

            Statement::Expression(statement) => {
                self.evaluate_expression(&statement.expression)?;

                Ok(Flow::Next)
            }

            Statement::Return(statement) => {
                let value = match &statement.argument {
                    Some(expression) => self.evaluate_expression(expression)?,

                    None => Value::Null,
                };

                Ok(Flow::Return(value))
            }

            Statement::Let(binding) => {
                let Pattern::Ident(pattern) = &binding.pattern else {
                    return Err(RuntimeError::UnsupportedStatement {
                        kind: "destructuring let",
                    });
                };

                let value = match &binding.init {
                    Some(expression) => self.evaluate_expression(expression)?,

                    None => Value::Null,
                };

                self.declare_variable(pattern.name.clone(), value)?;

                Ok(Flow::Next)
            }

            Statement::Block(block) => self.execute_block(block),

            Statement::If(_) => Err(RuntimeError::UnsupportedStatement { kind: "if" }),

            Statement::While(_) => Err(RuntimeError::UnsupportedStatement { kind: "while" }),

            Statement::ForIn(_) => Err(RuntimeError::UnsupportedStatement { kind: "for" }),

            Statement::Break(_) => Err(RuntimeError::UnsupportedStatement { kind: "break" }),

            Statement::Continue(_) => Err(RuntimeError::UnsupportedStatement { kind: "continue" }),

            Statement::Is(_) => Err(RuntimeError::UnsupportedStatement { kind: "is" }),

            Statement::Declaration(_) => Err(RuntimeError::UnsupportedStatement {
                kind: "nested declaration",
            }),
        }
    }

    fn evaluate_expression(&mut self, expression: &Expression) -> Result<Value, RuntimeError> {
        match expression {
            Expression::Literal(literal) => evaluate_literal(&literal.kind),

            Expression::Ident(identifier) => self.lookup_variable(&identifier.name),

            Expression::Call(call) => self.evaluate_call(call),

            Expression::Group(group) => self.evaluate_expression(&group.expression),

            Expression::Unary(_) => Err(RuntimeError::UnsupportedExpression { kind: "unary" }),

            Expression::Member(_) => Err(RuntimeError::UnsupportedExpression {
                kind: "member access",
            }),

            Expression::Index(_) => Err(RuntimeError::UnsupportedExpression { kind: "index" }),

            Expression::Assign(_) => {
                Err(RuntimeError::UnsupportedExpression { kind: "assignment" })
            }

            Expression::Block(_) => Err(RuntimeError::UnsupportedExpression { kind: "block" }),

            Expression::List(_) => Err(RuntimeError::UnsupportedExpression { kind: "list" }),

            Expression::Object(_) => Err(RuntimeError::UnsupportedExpression { kind: "object" }),

            Expression::Template(_) => {
                Err(RuntimeError::UnsupportedExpression { kind: "template" })
            }

            Expression::Closure(_) => Err(RuntimeError::UnsupportedExpression { kind: "closure" }),

            Expression::Binary(binary) => self.evaluate_binary(binary),

            Expression::DotIdent(_) => Err(RuntimeError::UnsupportedExpression {
                kind: "dot identifier",
            }),

            Expression::Is(_) => Err(RuntimeError::UnsupportedExpression { kind: "is" }),

            Expression::Component(_) => {
                Err(RuntimeError::UnsupportedExpression { kind: "component" })
            }
        }
    }

    fn evaluate_binary(&mut self, binary: &BinaryExpression) -> Result<Value, RuntimeError> {
        let left = self.evaluate_expression(&binary.left)?;

        let right = self.evaluate_expression(&binary.right)?;

        match (&binary.op, left, right) {
            (BinaryOp::Add, Value::String(left), Value::String(right)) => {
                Ok(Value::String(left + &right))
            }

            (BinaryOp::Add, Value::Number(left), Value::Number(right)) => {
                Ok(Value::Number(left + right))
            }

            (_, left, right) => Err(RuntimeError::TypeError {
                message: format!(
                    "binary operation is not supported for {} and {}",
                    value_type_name(&left),
                    value_type_name(&right),
                ),
            }),
        }
    }

    fn evaluate_call(&mut self, call: &CallExpression) -> Result<Value, RuntimeError> {
        let Expression::Ident(callee) = call.callee.as_ref() else {
            return Err(RuntimeError::UnsupportedExpression {
                kind: "non-identifier call",
            });
        };

        let mut arguments = Vec::with_capacity(call.args.len());

        for argument in &call.args {
            match argument {
                CallArg::Positional(expression) => {
                    arguments.push(self.evaluate_expression(expression)?);
                }

                CallArg::Named { .. } => {
                    return Err(RuntimeError::UnsupportedExpression {
                        kind: "named argument",
                    });
                }
            }
        }

        self.call_function(&callee.name, arguments)
    }

    fn declare_variable(&mut self, name: String, value: Value) -> Result<(), RuntimeError> {
        let scope = self.scopes.last_mut().ok_or(RuntimeError::NoActiveScope)?;

        scope.insert(name, value);

        Ok(())
    }

    fn lookup_variable(&self, name: &str) -> Result<Value, RuntimeError> {
        for scope in self.scopes.iter().rev() {
            if let Some(value) = scope.get(name) {
                return Ok(value.clone());
            }
        }

        Err(RuntimeError::UndefinedVariable {
            name: name.to_string(),
        })
    }
}

enum Flow {
    Next,
    Return(Value),
}

fn evaluate_literal(literal: &LiteralKind) -> Result<Value, RuntimeError> {
    match literal {
        LiteralKind::String(value) => Ok(Value::String(value.clone())),

        LiteralKind::Number(value) => {
            let number = value
                .0
                .parse::<f64>()
                .map_err(|_| RuntimeError::InvalidNumber {
                    value: value.0.clone(),
                })?;

            Ok(Value::Number(number))
        }

        LiteralKind::Boolean(value) => Ok(Value::Boolean(*value)),

        LiteralKind::Null => Ok(Value::Null),

        LiteralKind::Percent(_) => Err(RuntimeError::UnsupportedExpression {
            kind: "percent literal",
        }),
    }
}

fn native_function_name(function: &FunctionDeclaration) -> Result<Option<String>, RuntimeError> {
    let mut attributes = function
        .attributes
        .iter()
        .filter(|attribute| attribute.name == "native");

    let Some(attribute) = attributes.next() else {
        return Ok(None);
    };

    if attributes.next().is_some() {
        return Err(RuntimeError::InvalidNativeAttribute {
            function: function.name.clone(),
            message: "attribute appears more than once".to_string(),
        });
    }

    if attribute.args.len() != 1 {
        return Err(RuntimeError::InvalidNativeAttribute {
            function: function.name.clone(),
            message: "expected exactly one string argument".to_string(),
        });
    }

    let Expression::Literal(literal) = &attribute.args[0] else {
        return Err(RuntimeError::InvalidNativeAttribute {
            function: function.name.clone(),
            message: "argument must be a string literal".to_string(),
        });
    };

    let LiteralKind::String(name) = &literal.kind else {
        return Err(RuntimeError::InvalidNativeAttribute {
            function: function.name.clone(),
            message: "argument must be a string literal".to_string(),
        });
    };

    Ok(Some(name.clone()))
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::String(_) => "String",
        Value::Number(_) => "Number",
        Value::Boolean(_) => "Boolean",
        Value::Null => "Null",
    }
}
