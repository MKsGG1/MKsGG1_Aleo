// Copyright (C) 2019-2022 Aleo Systems Inc.
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

use super::*;

use leo_errors::{ParserError, Result};
use leo_span::sym;

impl ParserContext<'_> {
    /// Returns a [`ParsedInputFile`] struct filled with the data acquired in the file.
    pub fn parse_input(&mut self) -> Result<ParsedInputFile> {
        let mut sections = Vec::new();

        while self.has_next() {
            let token = self.peek()?;
            if matches!(token.token, Token::LeftSquare) {
                sections.push(self.parse_section()?);
            } else {
                return Err(ParserError::unexpected_token(token.token.clone(), &token.span).into());
            }
        }

        Ok(ParsedInputFile { sections })
    }

    /// Parses particular section in the Input or State file.
    /// `
    /// [<identifier>]
    /// <...definition>
    /// `
    /// Returns [`Section`].
    pub fn parse_section(&mut self) -> Result<Section> {
        self.expect(Token::LeftSquare)?;
        let section = self.expect_ident()?;
        self.expect(Token::RightSquare)?;

        let mut definitions = Vec::new();

        while let Some(SpannedToken {
            token: Token::Const | Token::Private | Token::Public | Token::Ident(_),
            ..
        }) = self.peek_option()
        {
            definitions.push(self.parse_input_definition(section.name == sym::main)?);
        }

        Ok(Section {
            name: section.name,
            span: section.span,
            definitions,
        })
    }

    /// Parses a single parameter definition:
    /// `<identifier> : <type> = <expression>;`
    /// Returns [`Definition`].
    pub fn parse_input_definition(&mut self, is_main: bool) -> Result<Definition> {
        let const_ = self.eat(Token::Const).is_some();
        let private = self.eat(Token::Private).is_some();
        let public = self.eat(Token::Public).is_some();

        match (const_, private, public) {
            (true, false, false) | (false, true, false) | (false, false, true) if is_main => {}
            (false, false, false) if is_main => return Err(ParserError::inputs_no_variable_type_specified().into()),
            _ if is_main => return Err(ParserError::inputs_multpe_variable_types_specified().into()),
            _ => {}
        }

        let name = self.expect_ident()?;
        self.expect(Token::Colon)?;
        let (type_, span) = self.parse_type()?;
        self.expect(Token::Assign)?;
        let value = self.parse_primary_expression()?;
        self.expect(Token::Semicolon)?;

        Ok(Definition {
            const_,
            private,
            public,
            name,
            type_,
            value,
            span,
        })
    }
}
