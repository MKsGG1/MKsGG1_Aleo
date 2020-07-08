//! Methods to enforce constraints on statements in a compiled Leo program.

use crate::{
    errors::StatementError,
    new_scope,
    program::ConstrainedProgram,
    value::ConstrainedValue,
    GroupType,
    Integer,
};
use leo_types::{
    Assignee,
    ConditionalNestedOrEndStatement,
    ConditionalStatement,
    Expression,
    Identifier,
    RangeOrExpression,
    Span,
    Statement,
    Type,
};

use snarkos_models::{
    curves::{Field, PrimeField},
    gadgets::{
        r1cs::ConstraintSystem,
        utilities::{boolean::Boolean, eq::ConditionalEqGadget, select::CondSelectGadget, uint::UInt32},
    },
};

impl<F: Field + PrimeField, G: GroupType<F>> ConstrainedProgram<F, G> {
    fn resolve_assignee(&mut self, scope: String, assignee: Assignee) -> String {
        match assignee {
            Assignee::Identifier(name) => new_scope(scope, name.to_string()),
            Assignee::Array(array, _index) => self.resolve_assignee(scope, *array),
            Assignee::CircuitField(circuit_name, _member) => self.resolve_assignee(scope, *circuit_name),
        }
    }

    fn get_mutable_assignee(
        &mut self,
        name: String,
        span: Span,
    ) -> Result<&mut ConstrainedValue<F, G>, StatementError> {
        // Check that assignee exists and is mutable
        Ok(match self.get_mut(&name) {
            Some(value) => match value {
                ConstrainedValue::Mutable(mutable_value) => mutable_value,
                _ => return Err(StatementError::immutable_assign(name, span)),
            },
            None => return Err(StatementError::undefined_variable(name, span)),
        })
    }

    fn mutate_array<CS: ConstraintSystem<F>>(
        &mut self,
        cs: &mut CS,
        file_scope: String,
        function_scope: String,
        indicator: Option<Boolean>,
        name: String,
        range_or_expression: RangeOrExpression,
        mut new_value: ConstrainedValue<F, G>,
        span: Span,
    ) -> Result<(), StatementError> {
        let condition = indicator.unwrap_or(Boolean::Constant(true));

        // Resolve index so we know if we are assigning to a single value or a range of values
        match range_or_expression {
            RangeOrExpression::Expression(index) => {
                let index = self.enforce_index(cs, file_scope.clone(), function_scope.clone(), index, span.clone())?;

                // Modify the single value of the array in place
                match self.get_mutable_assignee(name, span.clone())? {
                    ConstrainedValue::Array(old) => {
                        new_value.resolve_type(&vec![old[index].to_type(span.clone())?], span.clone())?;

                        let name_unique = format!("select {} {}:{}", new_value, span.line, span.start);
                        let selected_value = ConstrainedValue::conditionally_select(
                            cs.ns(|| name_unique),
                            &condition,
                            &new_value,
                            &old[index],
                        )
                        .map_err(|_| {
                            StatementError::select_fail(new_value.to_string(), old[index].to_string(), span)
                        })?;

                        old[index] = selected_value;
                    }
                    _ => return Err(StatementError::array_assign_index(span)),
                }
            }
            RangeOrExpression::Range(from, to) => {
                let from_index = match from {
                    Some(integer) => {
                        self.enforce_index(cs, file_scope.clone(), function_scope.clone(), integer, span.clone())?
                    }
                    None => 0usize,
                };
                let to_index_option = match to {
                    Some(integer) => Some(self.enforce_index(
                        cs,
                        file_scope.clone(),
                        function_scope.clone(),
                        integer,
                        span.clone(),
                    )?),
                    None => None,
                };

                // Modify the range of values of the array
                let old_array = self.get_mutable_assignee(name, span.clone())?;
                let new_array = match (old_array.clone(), new_value) {
                    (ConstrainedValue::Array(mut mutable), ConstrainedValue::Array(new)) => {
                        let to_index = to_index_option.unwrap_or(mutable.len());

                        mutable.splice(from_index..to_index, new.iter().cloned());
                        ConstrainedValue::Array(mutable)
                    }
                    _ => return Err(StatementError::array_assign_range(span)),
                };
                let name_unique = format!("select {} {}:{}", new_array, span.line, span.start);
                let selected_array =
                    ConstrainedValue::conditionally_select(cs.ns(|| name_unique), &condition, &new_array, old_array)
                        .map_err(|_| StatementError::select_fail(new_array.to_string(), old_array.to_string(), span))?;

                *old_array = selected_array;
            }
        }

        Ok(())
    }

    fn mutute_circuit_field<CS: ConstraintSystem<F>>(
        &mut self,
        cs: &mut CS,
        indicator: Option<Boolean>,
        circuit_name: String,
        object_name: Identifier,
        mut new_value: ConstrainedValue<F, G>,
        span: Span,
    ) -> Result<(), StatementError> {
        let condition = indicator.unwrap_or(Boolean::Constant(true));

        match self.get_mutable_assignee(circuit_name, span.clone())? {
            ConstrainedValue::CircuitExpression(_variable, members) => {
                // Modify the circuit value.field in place
                let matched_field = members.into_iter().find(|object| object.0 == object_name);

                match matched_field {
                    Some(object) => match &object.1 {
                        ConstrainedValue::Function(_circuit_identifier, function) => {
                            return Err(StatementError::immutable_circuit_function(
                                function.function_name.to_string(),
                                span,
                            ));
                        }
                        ConstrainedValue::Static(_value) => {
                            return Err(StatementError::immutable_circuit_function("static".into(), span));
                        }
                        _ => {
                            new_value.resolve_type(&vec![object.1.to_type(span.clone())?], span.clone())?;

                            let name_unique = format!("select {} {}:{}", new_value, span.line, span.start);
                            let selected_value = ConstrainedValue::conditionally_select(
                                cs.ns(|| name_unique),
                                &condition,
                                &new_value,
                                &object.1,
                            )
                            .map_err(|_| {
                                StatementError::select_fail(new_value.to_string(), object.1.to_string(), span)
                            })?;

                            object.1 = selected_value.to_owned();
                        }
                    },
                    None => return Err(StatementError::undefined_circuit_object(object_name.to_string(), span)),
                }
            }
            _ => return Err(StatementError::undefined_circuit(object_name.to_string(), span)),
        }

        Ok(())
    }

    fn enforce_assign_statement<CS: ConstraintSystem<F>>(
        &mut self,
        cs: &mut CS,
        file_scope: String,
        function_scope: String,
        indicator: Option<Boolean>,
        assignee: Assignee,
        expression: Expression,
        span: Span,
    ) -> Result<(), StatementError> {
        // Get the name of the variable we are assigning to
        let variable_name = self.resolve_assignee(function_scope.clone(), assignee.clone());

        // Evaluate new value
        let mut new_value =
            self.enforce_expression(cs, file_scope.clone(), function_scope.clone(), &vec![], expression)?;

        // Mutate the old value into the new value
        match assignee {
            Assignee::Identifier(_identifier) => {
                let condition = indicator.unwrap_or(Boolean::Constant(true));
                let old_value = self.get_mutable_assignee(variable_name.clone(), span.clone())?;

                new_value.resolve_type(&vec![old_value.to_type(span.clone())?], span.clone())?;

                let name_unique = format!("select {} {}:{}", new_value, span.line, span.start);
                let selected_value =
                    ConstrainedValue::conditionally_select(cs.ns(|| name_unique), &condition, &new_value, old_value)
                        .map_err(|_| StatementError::select_fail(new_value.to_string(), old_value.to_string(), span))?;

                *old_value = selected_value;

                Ok(())
            }
            Assignee::Array(_assignee, range_or_expression) => self.mutate_array(
                cs,
                file_scope,
                function_scope,
                indicator,
                variable_name,
                range_or_expression,
                new_value,
                span,
            ),
            Assignee::CircuitField(_assignee, object_name) => {
                self.mutute_circuit_field(cs, indicator, variable_name, object_name, new_value, span)
            }
        }
    }

    fn evaluate_branch<CS: ConstraintSystem<F>>(
        &mut self,
        cs: &mut CS,
        file_scope: String,
        function_scope: String,
        indicator: Option<Boolean>,
        statements: Vec<Statement>,
        return_types: Vec<Type>,
    ) -> Result<Vec<(Option<Boolean>, ConstrainedValue<F, G>)>, StatementError> {
        let mut results = vec![];
        // Evaluate statements. Only allow a single return argument to be returned.
        for statement in statements.iter() {
            let mut value = self.enforce_statement(
                cs,
                file_scope.clone(),
                function_scope.clone(),
                indicator.clone(),
                statement.clone(),
                return_types.clone(),
            )?;

            results.append(&mut value);
        }

        Ok(results)
    }

    /// Enforces a statements.conditional statement with one or more branches.
    /// Due to R1CS constraints, we must evaluate every branch to properly construct the circuit.
    /// At program execution, we will pass an `indicator bit` down to all child statements within each branch.
    /// The `indicator bit` will select that branch while keeping the constraint system satisfied.
    fn enforce_conditional_statement<CS: ConstraintSystem<F>>(
        &mut self,
        cs: &mut CS,
        file_scope: String,
        function_scope: String,
        indicator: Option<Boolean>,
        statement: ConditionalStatement,
        return_types: Vec<Type>,
        span: Span,
    ) -> Result<Vec<(Option<Boolean>, ConstrainedValue<F, G>)>, StatementError> {
        let statement_string = statement.to_string();
        let outer_indicator = indicator.unwrap_or(Boolean::Constant(true));

        let expected_types = vec![Type::Boolean];
        let inner_indicator = match self.enforce_expression(
            cs,
            file_scope.clone(),
            function_scope.clone(),
            &expected_types,
            statement.condition.clone(),
        )? {
            ConstrainedValue::Boolean(resolved) => resolved,
            value => return Err(StatementError::conditional_boolean(value.to_string(), span)),
        };

        // Determine nested branch 1 selection
        let outer_indicator_string = outer_indicator
            .get_value()
            .map(|b| b.to_string())
            .unwrap_or(format!("[allocated]"));
        let inner_indicator_string = inner_indicator
            .get_value()
            .map(|b| b.to_string())
            .unwrap_or(format!("[allocated]"));
        let branch_1_name = format!(
            "branch indicator 1 {} && {}",
            outer_indicator_string, inner_indicator_string
        );
        let branch_1_indicator = Boolean::and(
            &mut cs.ns(|| format!("branch 1 {} {}:{}", statement_string, span.line, span.start)),
            &outer_indicator,
            &inner_indicator,
        )
        .map_err(|_| StatementError::indicator_calculation(branch_1_name, span.clone()))?;

        let mut results = vec![];

        // Execute branch 1
        let mut branch_1_result = self.evaluate_branch(
            cs,
            file_scope.clone(),
            function_scope.clone(),
            Some(branch_1_indicator),
            statement.statements,
            return_types.clone(),
        )?;

        results.append(&mut branch_1_result);

        // Determine nested branch 2 selection
        let inner_indicator = inner_indicator.not();
        let inner_indicator_string = inner_indicator
            .get_value()
            .map(|b| b.to_string())
            .unwrap_or(format!("[allocated]"));
        let branch_2_name = format!(
            "branch indicator 2 {} && {}",
            outer_indicator_string, inner_indicator_string
        );
        let branch_2_indicator = Boolean::and(
            &mut cs.ns(|| format!("branch 2 {} {}:{}", statement_string, span.line, span.start)),
            &outer_indicator,
            &inner_indicator,
        )
        .map_err(|_| StatementError::indicator_calculation(branch_2_name, span.clone()))?;

        // Execute branch 2
        let mut branch_2_result = match statement.next {
            Some(next) => match next {
                ConditionalNestedOrEndStatement::Nested(nested) => self.enforce_conditional_statement(
                    cs,
                    file_scope,
                    function_scope,
                    Some(branch_2_indicator),
                    *nested,
                    return_types,
                    span,
                )?,
                ConditionalNestedOrEndStatement::End(statements) => self.evaluate_branch(
                    cs,
                    file_scope,
                    function_scope,
                    Some(branch_2_indicator),
                    statements,
                    return_types,
                )?,
            },
            None => vec![],
        };

        results.append(&mut branch_2_result);

        Ok(results)
    }

    fn enforce_for_statement<CS: ConstraintSystem<F>>(
        &mut self,
        cs: &mut CS,
        file_scope: String,
        function_scope: String,
        indicator: Option<Boolean>,
        index: Identifier,
        start: Expression,
        stop: Expression,
        statements: Vec<Statement>,
        return_types: Vec<Type>,
        span: Span,
    ) -> Result<Vec<(Option<Boolean>, ConstrainedValue<F, G>)>, StatementError> {
        let mut results = vec![];

        let from = self.enforce_index(cs, file_scope.clone(), function_scope.clone(), start, span.clone())?;
        let to = self.enforce_index(cs, file_scope.clone(), function_scope.clone(), stop, span.clone())?;

        for i in from..to {
            // Store index in current function scope.
            // For loop scope is not implemented.
            let index_name = new_scope(function_scope.clone(), index.to_string());

            self.store(
                index_name,
                ConstrainedValue::Integer(Integer::U32(UInt32::constant(i as u32))),
            );

            // Evaluate statements and possibly return early
            let name_unique = format!("for loop iteration {} {}:{}", i, span.line, span.start);
            let mut result = self.evaluate_branch(
                &mut cs.ns(|| name_unique),
                file_scope.clone(),
                function_scope.clone(),
                indicator,
                statements.clone(),
                return_types.clone(),
            )?;

            results.append(&mut result);
        }

        Ok(results)
    }

    fn enforce_assert_eq_statement<CS: ConstraintSystem<F>>(
        &mut self,
        cs: &mut CS,
        indicator: Option<Boolean>,
        left: &ConstrainedValue<F, G>,
        right: &ConstrainedValue<F, G>,
        span: Span,
    ) -> Result<(), StatementError> {
        let condition = indicator.unwrap_or(Boolean::Constant(true));
        let name_unique = format!("assert {} == {} {}:{}", left, right, span.line, span.start);
        let result = left.conditional_enforce_equal(cs.ns(|| name_unique), right, &condition);

        Ok(result.map_err(|_| StatementError::assertion_failed(left.to_string(), right.to_string(), span))?)
    }

    /// Enforce a program statement.
    /// Returns a Vector of (indicator, value) tuples.
    /// Each evaluated statement may execute of one or more statements that may return early.
    /// To indicate which of these return values to take we conditionally select that value with the indicator bit.
    pub(crate) fn enforce_statement<CS: ConstraintSystem<F>>(
        &mut self,
        cs: &mut CS,
        file_scope: String,
        function_scope: String,
        indicator: Option<Boolean>,
        statement: Statement,
        return_types: Vec<Type>,
    ) -> Result<Vec<(Option<Boolean>, ConstrainedValue<F, G>)>, StatementError> {
        let mut results = vec![];
        match statement {
            Statement::Return(expressions, span) => {
                let return_value = (
                    indicator,
                    self.enforce_return_statement(cs, file_scope, function_scope, expressions, return_types, span)?,
                );

                results.push(return_value);
            }
            Statement::Definition(declare, variable, expression, span) => {
                self.enforce_definition_statement(cs, file_scope, function_scope, declare, variable, expression, span)?;
            }
            Statement::Assign(variable, expression, span) => {
                self.enforce_assign_statement(cs, file_scope, function_scope, indicator, variable, expression, span)?;
            }
            Statement::MultipleAssign(variables, function, span) => {
                self.enforce_multiple_definition_statement(cs, file_scope, function_scope, variables, function, span)?;
            }
            Statement::Conditional(statement, span) => {
                let mut result = self.enforce_conditional_statement(
                    cs,
                    file_scope,
                    function_scope,
                    indicator,
                    statement,
                    return_types,
                    span,
                )?;

                results.append(&mut result);
            }
            Statement::For(index, start, stop, statements, span) => {
                let mut result = self.enforce_for_statement(
                    cs,
                    file_scope,
                    function_scope,
                    indicator,
                    index,
                    start,
                    stop,
                    statements,
                    return_types,
                    span,
                )?;

                results.append(&mut result);
            }
            Statement::AssertEq(left, right, span) => {
                let (resolved_left, resolved_right) =
                    self.enforce_binary_expression(cs, file_scope, function_scope, &vec![], left, right, span.clone())?;

                self.enforce_assert_eq_statement(cs, indicator, &resolved_left, &resolved_right, span)?;
            }
            Statement::Expression(expression, span) => {
                let expression_string = expression.to_string();
                let value = self.enforce_expression(cs, file_scope, function_scope, &vec![], expression)?;

                // handle empty return value cases
                match &value {
                    ConstrainedValue::Return(values) => {
                        if !values.is_empty() {
                            return Err(StatementError::unassigned(expression_string, span));
                        }
                    }
                    _ => return Err(StatementError::unassigned(expression_string, span)),
                }

                let result = (indicator, value);

                results.push(result);
            }
        };

        Ok(results)
    }
}
