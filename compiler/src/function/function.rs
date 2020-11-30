// Copyright (C) 2019-2020 Aleo Systems Inc.
// This file is part of the Leo library.

// The Leo library is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// The Leo library is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with the Leo library. If not, see <https://www.gnu.org/licenses/>.

//! Enforces constraints on a function in a compiled Leo program.

use crate::{
    errors::FunctionError,
    program::{new_scope, ConstrainedProgram},
    value::ConstrainedValue,
    GroupType,
};

use leo_ast::{Expression, Function, FunctionInput, Type};

use snarkos_models::{
    curves::{Field, PrimeField},
    gadgets::r1cs::ConstraintSystem,
};

impl<F: Field + PrimeField, G: GroupType<F>> ConstrainedProgram<F, G> {
    pub(crate) fn enforce_function<CS: ConstraintSystem<F>>(
        &mut self,
        cs: &mut CS,
        scope: &str,
        caller_scope: &str,
        function: Function,
        input: Vec<Expression>,
        declared_circuit_reference: &str,
    ) -> Result<ConstrainedValue<F, G>, FunctionError> {
        let function_name = new_scope(scope, function.get_name());

        // Store input values as new variables in resolved program
        for (input_model, input_expression) in function.input.iter().zip(input.into_iter()) {
            let (name, value) = match input_model {
                FunctionInput::InputKeyword(keyword) => {
                    let input_value =
                        self.enforce_function_input(cs, scope, caller_scope, &function_name, None, input_expression)?;

                    (keyword.to_string(), input_value)
                }
                FunctionInput::SelfKeyword(keyword) => {
                    let input_value =
                        self.enforce_function_input(cs, scope, caller_scope, &function_name, None, input_expression)?;

                    (keyword.to_string(), input_value)
                }
                FunctionInput::MutSelfKeyword(keyword) => {
                    let input_value =
                        self.enforce_function_input(cs, scope, caller_scope, &function_name, None, input_expression)?;

                    (keyword.to_string(), input_value)
                }
                FunctionInput::Variable(input_model) => {
                    // First evaluate input expression
                    let mut input_value = self.enforce_function_input(
                        cs,
                        scope,
                        caller_scope,
                        &function_name,
                        Some(input_model.type_.clone()),
                        input_expression,
                    )?;

                    if input_model.mutable {
                        input_value = ConstrainedValue::Mutable(Box::new(input_value))
                    }

                    (input_model.identifier.name.clone(), input_value)
                }
            };

            // Store input as variable with {function_name}_{input_name}
            let input_program_identifier = new_scope(&function_name, &name);
            self.store(input_program_identifier, value);
        }

        // Evaluate every statement in the function and save all potential results
        let mut results = vec![];

        for statement in function.statements.iter() {
            let mut result = self.enforce_statement(
                cs,
                scope,
                &function_name,
                None,
                statement.clone(),
                function.output.clone(),
                declared_circuit_reference,
            )?;

            results.append(&mut result);
        }

        // Conditionally select a result based on returned indicators
        let mut return_values = ConstrainedValue::Tuple(vec![]);

        Self::conditionally_select_result(cs, &mut return_values, results, &function.span)?;

        if let ConstrainedValue::Tuple(ref returns) = return_values {
            let return_types = match function.output {
                Some(Type::Tuple(types)) => types.len(),
                Some(_) => 1usize,
                None => 0usize,
            };

            if return_types != returns.len() {
                return Err(FunctionError::return_arguments_length(
                    return_types,
                    returns.len(),
                    function.span.clone(),
                ));
            }
        }

        Ok(return_values)
    }
}
