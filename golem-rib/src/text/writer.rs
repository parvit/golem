// Copyright 2024-2025 Golem Cloud
//
// Licensed under the Golem Source License v1.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://license.golem.cloud/LICENSE
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::expr::Expr;
use crate::{ArmPattern, CallType, InstanceCreationType, MatchArm, Range};
use std::fmt::Display;
use std::io::Write;

pub fn write_expr(expr: &Expr) -> Result<String, WriterError> {
    let mut buf = vec![];
    let mut writer = Writer::new(&mut buf);

    writer.write_expr(expr)?;

    String::from_utf8(buf)
        .map_err(|err| WriterError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, err)))
}

pub fn write_arm_pattern(arm_pattern: &ArmPattern) -> Result<String, WriterError> {
    let mut buf = vec![];
    let mut writer = Writer::new(&mut buf);

    internal::write_arm_pattern(arm_pattern, &mut writer)?;

    String::from_utf8(buf)
        .map_err(|err| WriterError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, err)))
}

struct Writer<W> {
    inner: W,
}

#[derive(Debug)]
pub enum WriterError {
    Io(std::io::Error),
}

impl From<std::io::Error> for WriterError {
    fn from(err: std::io::Error) -> Self {
        WriterError::Io(err)
    }
}

impl Display for WriterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriterError::Io(err) => write!(f, "IO error: {err}"),
        }
    }
}

impl<W: Write> Writer<W> {
    fn new(w: W) -> Self {
        Self { inner: w }
    }

    fn write_code_start(&mut self) -> Result<(), WriterError> {
        self.write_display("${")
    }

    fn write_code_end(&mut self) -> Result<(), WriterError> {
        self.write_display("}")
    }

    fn write_expr(&mut self, expr: &Expr) -> Result<(), WriterError> {
        match expr {
            Expr::Literal { value, .. } => {
                self.write_display("\"")?;
                self.write_str(value)?;
                self.write_display("\"")
            }
            Expr::Identifier {
                variable_id,
                type_annotation,
                ..
            } => {
                self.write_str(variable_id.name())?;
                if let Some(type_name) = type_annotation {
                    self.write_str(": ")?;
                    self.write_display(type_name)
                } else {
                    Ok(())
                }
            }

            Expr::GenerateWorkerName { .. } => Ok(()),

            Expr::Range { range, .. } => match range {
                Range::Range { from, to } => {
                    self.write_expr(from)?;
                    self.write_str("..")?;
                    self.write_expr(to)
                }
                Range::RangeInclusive { from, to } => {
                    self.write_expr(from)?;
                    self.write_str("..=")?;
                    self.write_expr(to)
                }
                Range::RangeFrom { from } => {
                    self.write_str("..")?;
                    self.write_expr(from)
                }
            },

            Expr::Let {
                variable_id,
                type_annotation,
                expr,
                ..
            } => {
                self.write_str("let ")?;
                self.write_str(variable_id.name())?;
                if let Some(type_name) = type_annotation {
                    self.write_str(": ")?;
                    self.write_display(type_name)?;
                };
                self.write_str(" = ")?;
                self.write_expr(expr)
            }
            Expr::SelectField {
                expr,
                field,
                type_annotation,
                ..
            } => {
                self.write_expr(expr)?;
                self.write_str(".")?;
                self.write_str(field)?;
                if let Some(type_name) = type_annotation {
                    self.write_str(": ")?;
                    self.write_display(type_name)
                } else {
                    Ok(())
                }
            }
            Expr::SelectIndex {
                expr,
                index,
                type_annotation,
                ..
            } => {
                self.write_expr(expr)?;
                self.write_str("[")?;
                self.write_expr(index)?;
                self.write_str("]")?;
                if let Some(type_name) = type_annotation {
                    self.write_str(": ")?;
                    self.write_display(type_name)
                } else {
                    Ok(())
                }
            }

            Expr::Sequence {
                exprs,
                type_annotation,
                ..
            } => {
                self.write_display("[")?;
                for (idx, expr) in exprs.iter().enumerate() {
                    if idx != 0 {
                        self.write_display(",")?;
                        self.write_display(" ")?;
                    }
                    self.write_expr(expr)?;
                }
                self.write_display("]")?;
                if let Some(type_name) = type_annotation {
                    self.write_str(": ")?;
                    self.write_display(type_name)
                } else {
                    Ok(())
                }
            }
            Expr::Record { exprs, .. } => {
                self.write_display("{")?;
                for (idx, (key, value)) in exprs.iter().enumerate() {
                    if idx != 0 {
                        self.write_display(",")?;
                        self.write_display(" ")?;
                    }
                    self.write_str(key)?;
                    self.write_display(":")?;
                    self.write_display(" ")?;
                    self.write_expr(value)?;
                }
                self.write_display("}")
            }
            Expr::Tuple { exprs, .. } => {
                self.write_display("(")?;
                for (idx, expr) in exprs.iter().enumerate() {
                    if idx != 0 {
                        self.write_display(",")?;
                        self.write_display(" ")?;
                    }
                    self.write_expr(expr)?;
                }
                self.write_display(")")
            }
            Expr::Number {
                number,
                type_annotation,
                ..
            } => {
                self.write_display(number.value.to_string())?;
                if let Some(type_name) = type_annotation {
                    self.write_str(": ")?;
                    self.write_display(type_name)?;
                }
                Ok(())
            }
            Expr::Flags { flags, .. } => {
                self.write_display("{")?;
                for (idx, flag) in flags.iter().enumerate() {
                    if idx != 0 {
                        self.write_display(",")?;
                        self.write_display(" ")?;
                    }
                    self.write_str(flag)?;
                }
                self.write_display("}")
            }
            Expr::Boolean { value, .. } => self.write_display(value),
            Expr::Concat { exprs, .. } => {
                self.write_display("\"")?;
                internal::write_concatenated_exprs(self, exprs)?;
                self.write_display("\"")
            }
            Expr::ExprBlock { exprs, .. } => {
                for (idx, expr) in exprs.iter().enumerate() {
                    if idx != 0 {
                        self.write_display(";")?;
                        self.write_display("\n")?;
                    }
                    self.write_expr(expr)?;
                }
                Ok(())
            }
            Expr::Not { expr, .. } => {
                self.write_str("!")?;
                self.write_expr(expr)
            }
            Expr::GreaterThan { lhs, rhs, .. } => {
                self.write_expr(lhs)?;
                self.write_str(" > ")?;
                self.write_expr(rhs)
            }
            Expr::Plus { lhs, rhs, .. } => {
                self.write_expr(lhs)?;
                self.write_str(" + ")?;
                self.write_expr(rhs)
            }
            Expr::Minus { lhs, rhs, .. } => {
                self.write_expr(lhs)?;
                self.write_str(" - ")?;
                self.write_expr(rhs)
            }
            Expr::Divide { lhs, rhs, .. } => {
                self.write_expr(lhs)?;
                self.write_str(" / ")?;
                self.write_expr(rhs)
            }
            Expr::Multiply { lhs, rhs, .. } => {
                self.write_expr(lhs)?;
                self.write_str(" * ")?;
                self.write_expr(rhs)
            }
            Expr::GreaterThanOrEqualTo { lhs, rhs, .. } => {
                self.write_expr(lhs)?;
                self.write_str(" >= ")?;
                self.write_expr(rhs)
            }
            Expr::LessThanOrEqualTo { lhs, rhs, .. } => {
                self.write_expr(lhs)?;
                self.write_str(" <= ")?;
                self.write_expr(rhs)
            }
            Expr::EqualTo { lhs, rhs, .. } => {
                self.write_expr(lhs)?;
                self.write_str(" == ")?;
                self.write_expr(rhs)
            }
            Expr::LessThan { lhs, rhs, .. } => {
                self.write_expr(lhs)?;
                self.write_str(" < ")?;
                self.write_expr(rhs)
            }
            Expr::Cond { cond, lhs, rhs, .. } => {
                self.write_str("if ")?;
                self.write_expr(cond)?;
                self.write_str(" then ")?;
                self.write_expr(lhs)?;
                self.write_str(" else ")?;
                self.write_expr(rhs)
            }
            Expr::PatternMatch {
                predicate,
                match_arms,
                ..
            } => {
                self.write_str("match ")?;
                self.write_expr(predicate)?;
                self.write_str(" { ")?;
                self.write_display(" ")?;
                for (idx, match_term) in match_arms.iter().enumerate() {
                    if idx != 0 {
                        self.write_str(", ")?;
                    }
                    let MatchArm {
                        arm_pattern,
                        arm_resolution_expr,
                    } = &match_term;
                    internal::write_arm_pattern(arm_pattern, self)?;
                    self.write_str(" => ")?;
                    self.write_expr(arm_resolution_expr)?;
                }
                self.write_str(" } ")
            }
            Expr::Option {
                expr,
                type_annotation,
                ..
            } => {
                match expr {
                    Some(expr) => {
                        self.write_str("some(")?;
                        self.write_expr(expr)?;
                        self.write_str(")")?;
                    }
                    None => self.write_str("none")?,
                }

                if let Some(type_name) = type_annotation {
                    self.write_str(": ")?;
                    self.write_display(type_name)
                } else {
                    Ok(())
                }
            }
            Expr::Result { expr, .. } => match expr {
                Ok(expr) => {
                    self.write_str("ok(")?;
                    self.write_expr(expr)?;
                    self.write_str(")")
                }
                Err(expr) => {
                    self.write_str("err(")?;
                    self.write_expr(expr)?;
                    self.write_str(")")
                }
            },

            Expr::Call {
                call_type,
                generic_type_parameter,
                args,
                ..
            } => {
                let function_name = match call_type {
                    CallType::Function { function_name, .. } => {
                        function_name.function.name_pretty()
                    }
                    CallType::VariantConstructor(name) => name.to_string(),
                    CallType::EnumConstructor(name) => name.to_string(),
                    CallType::InstanceCreation(instance) => match instance {
                        InstanceCreationType::WitWorker { .. } => "instance".to_string(),
                        InstanceCreationType::WitResource { resource_name, .. } => {
                            resource_name.resource_name.to_string()
                        }
                    },
                };

                self.write_str(function_name)?;

                if let Some(type_parameter) = generic_type_parameter {
                    self.write_str("[")?;
                    self.write_str(type_parameter.value.clone())?;
                    self.write_str("]")?;
                }

                match call_type {
                    CallType::Function { .. } | CallType::InstanceCreation(_) => {
                        self.write_display("(")?;
                        for (idx, param) in args.iter().enumerate() {
                            if idx != 0 {
                                self.write_display(",")?;
                                self.write_display(" ")?;
                            }
                            self.write_expr(param)?;
                        }
                        self.write_display(")")
                    }
                    CallType::VariantConstructor(_) => {
                        if !args.is_empty() {
                            self.write_str("(")?;
                            for (idx, param) in args.iter().enumerate() {
                                if idx != 0 {
                                    self.write_display(",")?;
                                    self.write_display(" ")?;
                                }
                                self.write_expr(param)?;
                            }
                            self.write_str(")")
                        } else {
                            Ok(())
                        }
                    }

                    CallType::EnumConstructor(_) => Ok(()),
                }
            }

            Expr::Unwrap { expr, .. } => {
                self.write_str("unwrap(")?;
                self.write_expr(expr)?;
                self.write_str(")")
            }

            Expr::Length { expr, .. } => {
                self.write_str("len(")?;
                self.write_expr(expr)?;
                self.write_str(")")
            }

            Expr::Throw { message, .. } => {
                self.write_str("throw(")?;
                self.write_str(message)?;
                self.write_str(")")
            }
            Expr::GetTag { expr, .. } => {
                self.write_str("get_tag(")?;
                self.write_expr(expr)?;
                self.write_str(")")
            }
            Expr::And { lhs, rhs, .. } => {
                self.write_expr(lhs)?;
                self.write_str(" && ")?;
                self.write_expr(rhs)
            }
            Expr::Or { lhs, rhs, .. } => {
                self.write_expr(lhs)?;
                self.write_str(" || ")?;
                self.write_expr(rhs)
            }
            Expr::ListComprehension {
                iterated_variable,
                iterable_expr,
                yield_expr,
                ..
            } => {
                self.write_display(" for ")?;
                self.write_display(iterated_variable.to_string())?;
                self.write_display(" in ")?;
                self.write_expr(iterable_expr)?;
                self.write_display(" { ")?;
                self.write_display("\n")?;
                internal::write_yield_block(self, yield_expr)?;
                self.write_display(";")?;
                self.write_display(" } ")
            }

            Expr::ListReduce {
                reduce_variable,
                iterated_variable,
                iterable_expr,
                init_value_expr,
                yield_expr,
                ..
            } => {
                self.write_display("reduce ")?;
                self.write_display(reduce_variable.to_string())?;
                self.write_display(", ")?;
                self.write_display(iterated_variable.to_string())?;
                self.write_display(" in ")?;
                self.write_expr(iterable_expr)?;
                self.write_display(" from ")?;
                self.write_expr(init_value_expr)?;
                self.write_display(" { ")?;
                self.write_display("\n")?;
                internal::write_yield_block(self, yield_expr)?;
                self.write_display(" } ")
            }

            Expr::InvokeMethodLazy {
                lhs,
                method,
                generic_type_parameter,
                args,
                ..
            } => {
                self.write_expr(lhs)?;
                self.write_str(".")?;
                self.write_str(method)?;
                if let Some(type_parameter) = generic_type_parameter {
                    self.write_str("[")?;
                    self.write_str(type_parameter.value.clone())?;
                    self.write_str("]")?;
                }
                self.write_display("(")?;
                for (idx, param) in args.iter().enumerate() {
                    if idx != 0 {
                        self.write_display(",")?;
                        self.write_display(" ")?;
                    }
                    self.write_expr(param)?;
                }
                self.write_display(")")
            }
        }
    }

    fn write_str(&mut self, s: impl AsRef<str>) -> Result<(), WriterError> {
        self.inner.write_all(s.as_ref().as_bytes())?;
        Ok(())
    }

    fn write_display(&mut self, d: impl std::fmt::Display) -> Result<(), WriterError> {
        write!(self.inner, "{d}")?;
        Ok(())
    }
}

mod internal {
    use crate::expr::{ArmPattern, Expr};
    use crate::text::writer::{Writer, WriterError};

    pub(crate) enum ExprType<'a> {
        Code(&'a Expr),
        Text(&'a str),
        StringInterpolated,
    }

    pub(crate) fn write_yield_block<W>(
        writer: &mut Writer<W>,
        expr: &Expr,
    ) -> Result<(), WriterError>
    where
        W: std::io::Write,
    {
        if let Expr::ExprBlock { exprs, .. } = expr {
            let last_line_index = exprs.len() - 1;

            for (index, line) in exprs.iter().enumerate() {
                if index == last_line_index {
                    writer.write_display("yield ")?;
                    writer.write_expr(line)?;
                } else {
                    writer.write_expr(line)?;
                }

                writer.write_display("\n")?;
            }

            Ok(())
        } else {
            writer.write_display("yield ")?;
            writer.write_expr(expr)
        }
    }

    pub(crate) fn get_expr_type(expr: &Expr) -> ExprType {
        match expr {
            Expr::Literal { value, .. } => ExprType::Text(value),
            Expr::Concat { .. } => ExprType::StringInterpolated,
            expr => ExprType::Code(expr),
        }
    }

    // Only to make sure that we are not wrapping literals with quotes - intercepting
    // the logic within the writer for ExprType::Code
    pub(crate) fn write_concatenated_exprs<W>(
        writer: &mut Writer<W>,
        exprs: &[Expr],
    ) -> Result<(), WriterError>
    where
        W: std::io::Write,
    {
        for expr in exprs.iter() {
            match get_expr_type(expr) {
                ExprType::Text(text) => {
                    writer.write_str(text)?;
                }
                ExprType::Code(expr) => {
                    writer.write_code_start()?;
                    writer.write_expr(expr)?;
                    writer.write_code_end()?;
                }
                ExprType::StringInterpolated => {
                    writer.write_code_start()?;
                    writer.write_expr(expr)?;
                    writer.write_code_end()?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn write_arm_pattern<W>(
        match_case: &ArmPattern,
        writer: &mut Writer<W>,
    ) -> Result<(), WriterError>
    where
        W: std::io::Write,
    {
        match match_case {
            ArmPattern::WildCard => writer.write_str("_"),
            ArmPattern::As(name, pattern) => {
                writer.write_str(name)?;
                writer.write_str(" @ ")?;
                write_arm_pattern(pattern, writer)
            }
            ArmPattern::Constructor(constructor_type, variables) => {
                if !variables.is_empty() {
                    writer.write_display(constructor_type)?;

                    writer.write_str("(")?;

                    for (idx, pattern) in variables.iter().enumerate() {
                        if idx != 0 {
                            writer.write_str(",")?;
                        }
                        write_arm_pattern(pattern, writer)?;
                    }

                    writer.write_str(")")
                } else {
                    writer.write_display(constructor_type)
                }
            }

            ArmPattern::TupleConstructor(variables) => {
                writer.write_str("(")?;

                for (idx, pattern) in variables.iter().enumerate() {
                    if idx != 0 {
                        writer.write_str(",")?;
                    }
                    write_arm_pattern(pattern, writer)?;
                }

                writer.write_str(")")
            }

            ArmPattern::ListConstructor(patterns) => {
                writer.write_str("[")?;

                for (idx, pattern) in patterns.iter().enumerate() {
                    if idx != 0 {
                        writer.write_str(",")?;
                    }
                    write_arm_pattern(pattern, writer)?;
                }

                writer.write_str("]")
            }

            ArmPattern::RecordConstructor(fields) => {
                writer.write_str("{")?;

                for (idx, (key, value)) in fields.iter().enumerate() {
                    if idx != 0 {
                        writer.write_str(",")?;
                    }
                    writer.write_str(key)?;
                    writer.write_str(":")?;
                    write_arm_pattern(value, writer)?;
                }

                writer.write_str("}")
            }

            ArmPattern::Literal(expr) => match *expr.clone() {
                Expr::Identifier { variable_id, .. } => writer.write_str(variable_id.name()),
                any_expr => writer.write_expr(&any_expr),
            },
        }
    }
}
