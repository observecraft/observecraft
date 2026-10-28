//! SQL Editor: checks a user's SQL and rewrites it onto org-scoped tables.
//!
//! A query is parsed with `sqlparser` (ClickHouse dialect) and walked with an allowlist: one
//! SELECT (WITH, JOIN, UNION, GROUP BY, ORDER BY, LIMIT, window functions), pure functions from
//! [`ALLOWED_FUNCTIONS`], and tables from the caller's list of virtual tables. Everything else,
//! including table functions, SETTINGS, FORMAT and INTO, is refused. The caller turns each virtual
//! table into a subquery that filters on the org, so nothing in the user's SQL can name a stored
//! table. String literals are bound as parameters instead of being rendered back, so quoting
//! differences between the parser and ClickHouse can't change what runs.

use sqlparser::ast::{
    AccessExpr, ArrayElemTypeDef, BinaryOperator, CastKind, CeilFloorKind, DataType, DateTimeField, Distinct, Expr, Function, FunctionArg, FunctionArgExpr, FunctionArgumentList, FunctionArguments,
    GroupByExpr, GroupByWithModifier, Ident, Interval, JoinConstraint, JoinOperator, LambdaFunction, LambdaFunctionParameter, LambdaSyntax, LimitClause, NamedWindowDefinition, NamedWindowExpr,
    ObjectName, ObjectNamePart, OneOrManyWithParens, OrderBy, OrderByExpr, OrderByKind, OrderByOptions, OrderBySort, Query, Select, SelectFlavor, SelectItem, SelectItemQualifiedWildcardKind, SetExpr,
    SetOperator, SetQuantifier, Statement, Subscript, TableAlias, TableFactor, TableWithJoins, UnaryOperator, Value, ValueWithSpan, WildcardAdditionalOptions, WindowFrameBound, WindowSpec,
    WindowType,
};
use sqlparser::dialect::ClickHouseDialect;
use sqlparser::parser::Parser;

/// A part of a checked query, in order. The caller renders `Text` as is, binds each `Literal` as a
/// parameter (`?`) and replaces each `Table` with the org-scoped subquery of that virtual table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqlPiece {
    Text(String),
    Literal(String),
    Table(String),
}

/// A query that passed [`check_sql`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedSql {
    pub pieces: Vec<SqlPiece>,
    /// The virtual tables it reads, each once, in order of appearance.
    pub tables: Vec<String>,
}

/// Why a query was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlError(pub String);

impl std::fmt::Display for SqlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Longest query accepted, in characters.
pub const MAX_SQL_CHARS: usize = 100_000;
/// Nesting the parser follows before giving up (stack safety).
const RECURSION_LIMIT: usize = 64;
const MAX_IDENT_CHARS: usize = 128;
/// Markers put into the AST and cut out of the rendered text. User identifiers can't contain `__oc`.
const MARK: &str = "__oc_";

/// Functions a query may call (ClickHouse names, matched case-insensitively). Only pure functions:
/// nothing that reads tables, files, the network, settings or server state.
pub const ALLOWED_FUNCTIONS: &[&str] = &[
    // aggregates
    "count",
    "sum",
    "avg",
    "min",
    "max",
    "any",
    "anyLast",
    "argMin",
    "argMax",
    "uniq",
    "uniqExact",
    "groupArray",
    "groupUniqArray",
    "quantile",
    "quantiles",
    "quantileExact",
    "quantileTDigest",
    "median",
    "topK",
    "stddevPop",
    "stddevSamp",
    "varPop",
    "varSamp",
    "countIf",
    "sumIf",
    "avgIf",
    "minIf",
    "maxIf",
    "uniqIf",
    "uniqExactIf",
    "quantileIf",
    "anyIf",
    "argMaxIf",
    "argMinIf",
    "groupArrayIf",
    // window
    "row_number",
    "rank",
    "dense_rank",
    "percent_rank",
    "first_value",
    "last_value",
    "lagInFrame",
    "leadInFrame",
    "ntile",
    // conditionals and nulls
    "if",
    "multiIf",
    "coalesce",
    "ifNull",
    "nullIf",
    "isNull",
    "isNotNull",
    "assumeNotNull",
    "greatest",
    "least",
    // dates and times
    "now",
    "now64",
    "today",
    "yesterday",
    "toDate",
    "toDateTime",
    "toDateTime64",
    "toStartOfMinute",
    "toStartOfFiveMinutes",
    "toStartOfFifteenMinutes",
    "toStartOfHour",
    "toStartOfDay",
    "toStartOfWeek",
    "toStartOfMonth",
    "toStartOfYear",
    "toStartOfInterval",
    "date_trunc",
    "dateTrunc",
    "toUnixTimestamp",
    "toUnixTimestamp64Milli",
    "fromUnixTimestamp",
    "fromUnixTimestamp64Milli",
    "dateDiff",
    "date_diff",
    "age",
    "addMinutes",
    "addHours",
    "addDays",
    "subtractMinutes",
    "subtractHours",
    "subtractDays",
    "formatDateTime",
    "toYear",
    "toMonth",
    "toDayOfWeek",
    "toHour",
    "toMinute",
    "parseDateTimeBestEffort",
    "parseDateTimeBestEffortOrNull",
    "toIntervalSecond",
    "toIntervalMinute",
    "toIntervalHour",
    "toIntervalDay",
    // strings
    "lower",
    "upper",
    "lowerUTF8",
    "upperUTF8",
    "length",
    "lengthUTF8",
    "concat",
    "substring",
    "substr",
    "position",
    "positionCaseInsensitive",
    "startsWith",
    "endsWith",
    "trim",
    "trimBoth",
    "trimLeft",
    "trimRight",
    "replaceAll",
    "replaceOne",
    "replaceRegexpAll",
    "replaceRegexpOne",
    "match",
    "extract",
    "extractAll",
    "splitByChar",
    "splitByString",
    "arrayStringConcat",
    "leftPad",
    "rightPad",
    "left",
    "right",
    "reverse",
    "format",
    "toString",
    "like",
    "notLike",
    "ilike",
    "notILike",
    "multiSearchAny",
    "empty",
    "notEmpty",
    // numbers and conversions
    "toInt32",
    "toInt64",
    "toUInt32",
    "toUInt64",
    "toFloat64",
    "toInt64OrZero",
    "toInt64OrNull",
    "toFloat64OrZero",
    "toFloat64OrNull",
    "toDecimal64",
    "round",
    "floor",
    "ceil",
    "abs",
    "sqrt",
    "log",
    "log2",
    "log10",
    "exp",
    "pow",
    "power",
    "intDiv",
    "modulo",
    // arrays, maps, tuples
    "array",
    "tuple",
    "map",
    "arrayJoin",
    "has",
    "hasAny",
    "hasAll",
    "indexOf",
    "arrayMap",
    "arrayFilter",
    "arrayExists",
    "arrayCount",
    "arraySort",
    "arrayReverseSort",
    "arrayDistinct",
    "arraySlice",
    "arrayElement",
    "arrayConcat",
    "arrayFirst",
    "arrayUniq",
    "arraySum",
    "mapKeys",
    "mapValues",
    "mapContains",
    "tupleElement",
    // JSON
    "JSONExtractString",
    "JSONExtractInt",
    "JSONExtractFloat",
    "JSONExtractBool",
    "JSONExtractRaw",
    "JSONHas",
    "JSONLength",
    "JSONExtractKeys",
    // formatting
    "formatReadableSize",
    "formatReadableQuantity",
    "formatReadableTimeSpan",
    "toTypeName",
];

/// Checks one SELECT and rewrites it: every table it names must be one of `tables` (or a CTE of the
/// query) and becomes a [`SqlPiece::Table`]; string literals become [`SqlPiece::Literal`]s; CTEs get
/// generated names. The result reads at most `max_rows + 1` rows, so the caller can tell it was cut.
///
/// The rewritten text is rendered from the syntax tree, so comments and the user's spelling never
/// reach ClickHouse; whatever the checker doesn't know is refused.
pub fn check_sql(sql: &str, tables: &[&str], max_rows: u64) -> Result<CheckedSql, SqlError> {
    if sql.chars().count() > MAX_SQL_CHARS {
        return Err(err(format!("Queries are at most {MAX_SQL_CHARS} characters")));
    }
    if sql.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
        return Err(err("The query contains control characters"));
    }
    if sql.trim().trim_end_matches(';').trim().is_empty() {
        return Err(err("Enter a query"));
    }
    let dialect = ClickHouseDialect {};
    let mut statements = Parser::new(&dialect).with_recursion_limit(RECURSION_LIMIT).try_with_sql(sql).and_then(|mut p| p.parse_statements()).map_err(|e| err(format!("Syntax error: {e}")))?;
    if statements.len() != 1 {
        return Err(err("Run one statement at a time"));
    }
    let Statement::Query(mut query) = statements.remove(0) else {
        return Err(err("Only SELECT queries can run here"));
    };
    let mut rw = Rewriter { tables, scopes: Vec::new(), literals: Vec::new(), used: Vec::new(), ctes: 0 };
    rw.query(&mut query)?;
    let text = if cap(&mut query, max_rows) { format!("SELECT * FROM ({query}) LIMIT {}", max_rows + 1) } else { query.to_string() };
    let pieces = split(&text, &rw.literals, &rw.used)?;
    Ok(CheckedSql { pieces, tables: rw.used })
}

fn err(msg: impl Into<String>) -> SqlError {
    SqlError(msg.into())
}

type Checked = Result<(), SqlError>;

struct Rewriter<'a> {
    tables: &'a [&'a str],
    /// CTEs visible at this point, innermost last: (name in the query, generated name).
    scopes: Vec<Vec<(String, String)>>,
    literals: Vec<String>,
    used: Vec<String>,
    ctes: usize,
}

impl Rewriter<'_> {
    fn query(&mut self, q: &mut Query) -> Checked {
        let Query { with, body, order_by, limit_clause, fetch, locks, for_clause, settings, format_clause, pipe_operators } = q;
        if settings.is_some() {
            return Err(err("SETTINGS isn't allowed: limits are set by the server"));
        }
        if format_clause.is_some() {
            return Err(err("FORMAT isn't allowed: results come back as a table"));
        }
        if fetch.is_some() || !locks.is_empty() || for_clause.is_some() || !pipe_operators.is_empty() {
            return Err(err("FETCH, FOR and pipe clauses aren't supported"));
        }
        let scoped = with.is_some();
        if let Some(with) = with {
            if with.recursive {
                return Err(err("Recursive CTEs aren't supported"));
            }
            self.scopes.push(Vec::new());
            for cte in &mut with.cte_tables {
                if cte.materialized.is_some() || cte.from.is_some() || !cte.alias.columns.is_empty() || cte.alias.at.is_some() {
                    return Err(err("CTEs are written `name AS (SELECT …)`"));
                }
                let mut name = cte.alias.name.clone();
                ident(&mut name)?;
                // The body is checked before the name is visible: a CTE can't read itself.
                self.query(&mut cte.query)?;
                let generated = format!("{MARK}cte_{}", self.ctes);
                self.ctes += 1;
                cte.alias = TableAlias { explicit: false, name: Ident::with_quote('`', generated.clone()), columns: Vec::new(), at: None };
                if let Some(scope) = self.scopes.last_mut() {
                    scope.push((name.value, generated));
                }
            }
        }
        let checked = self.set_expr(body).and_then(|_| order_by.as_mut().map_or(Ok(()), |o| self.order_by(o))).and_then(|_| limit_clause.as_mut().map_or(Ok(()), |l| self.limit(l)));
        if scoped {
            self.scopes.pop();
        }
        checked
    }

    fn set_expr(&mut self, s: &mut SetExpr) -> Checked {
        match s {
            SetExpr::Select(select) => self.select(select),
            SetExpr::Query(q) => self.query(q),
            SetExpr::SetOperation { left, op, set_quantifier, right } => {
                if !matches!(op, SetOperator::Union | SetOperator::Except | SetOperator::Intersect) {
                    return Err(err(format!("{op} isn't supported")));
                }
                if !matches!(set_quantifier, SetQuantifier::None | SetQuantifier::All | SetQuantifier::Distinct) {
                    return Err(err(format!("{op} {set_quantifier} isn't supported")));
                }
                self.set_expr(left)?;
                self.set_expr(right)
            }
            _ => Err(err("Only SELECT queries can run here")),
        }
    }

    fn select(&mut self, s: &mut Select) -> Checked {
        let Select {
            select_token: _,
            optimizer_hints,
            distinct,
            select_modifiers,
            top,
            top_before_distinct: _,
            projection,
            exclude,
            into,
            from,
            lateral_views,
            prewhere,
            selection,
            connect_by,
            group_by,
            cluster_by,
            distribute_by,
            sort_by,
            having,
            named_window,
            qualify,
            window_before_qualify: _,
            value_table_mode,
            flavor,
        } = s;
        if into.is_some() {
            return Err(err("SELECT … INTO isn't allowed"));
        }
        if !optimizer_hints.is_empty()
            || select_modifiers.as_ref().is_some_and(|m| m.is_any_set())
            || top.is_some()
            || exclude.is_some()
            || !lateral_views.is_empty()
            || !connect_by.is_empty()
            || !cluster_by.is_empty()
            || !distribute_by.is_empty()
            || !sort_by.is_empty()
            || value_table_mode.is_some()
            || *flavor != SelectFlavor::Standard
        {
            return Err(err("This SELECT syntax isn't supported; use standard SELECT … FROM … WHERE …"));
        }
        for t in from.iter_mut() {
            self.table_with_joins(t)?;
        }
        match distinct {
            None | Some(Distinct::All) | Some(Distinct::Distinct) => {}
            Some(Distinct::On(exprs)) => self.exprs(exprs)?,
        }
        for item in projection.iter_mut() {
            self.select_item(item)?;
        }
        for e in [prewhere, selection, having, qualify].into_iter().flatten() {
            self.expr(e)?;
        }
        match group_by {
            GroupByExpr::All(mods) => group_modifiers(mods)?,
            GroupByExpr::Expressions(exprs, mods) => {
                self.exprs(exprs)?;
                group_modifiers(mods)?;
            }
        }
        for NamedWindowDefinition(name, w) in named_window.iter_mut() {
            ident(name)?;
            match w {
                NamedWindowExpr::NamedWindow(id) => ident(id)?,
                NamedWindowExpr::WindowSpec(spec) => self.window(spec)?,
            }
        }
        Ok(())
    }

    fn select_item(&mut self, item: &mut SelectItem) -> Checked {
        match item {
            SelectItem::UnnamedExpr(e) => self.expr(e),
            SelectItem::ExprWithAlias { expr, alias } => {
                ident(alias)?;
                self.expr(expr)
            }
            SelectItem::Wildcard(opts) => wildcard_options(opts),
            SelectItem::QualifiedWildcard(SelectItemQualifiedWildcardKind::ObjectName(name), opts) => {
                column_path(name)?;
                wildcard_options(opts)
            }
            _ => Err(err("This select item isn't supported")),
        }
    }

    fn table_with_joins(&mut self, t: &mut TableWithJoins) -> Checked {
        self.table_factor(&mut t.relation)?;
        for join in &mut t.joins {
            if join.global {
                return Err(err("GLOBAL JOIN isn't supported"));
            }
            let constraint = match &mut join.join_operator {
                JoinOperator::Join(c)
                | JoinOperator::Inner(c)
                | JoinOperator::Left(c)
                | JoinOperator::LeftOuter(c)
                | JoinOperator::Right(c)
                | JoinOperator::RightOuter(c)
                | JoinOperator::FullOuter(c)
                | JoinOperator::CrossJoin(c)
                | JoinOperator::Semi(c)
                | JoinOperator::LeftSemi(c)
                | JoinOperator::RightSemi(c)
                | JoinOperator::Anti(c)
                | JoinOperator::LeftAnti(c)
                | JoinOperator::RightAnti(c) => c,
                JoinOperator::ArrayJoin | JoinOperator::LeftArrayJoin | JoinOperator::InnerArrayJoin => return Err(err("ARRAY JOIN isn't supported; use arrayJoin()")),
                _ => return Err(err("This kind of JOIN isn't supported")),
            };
            self.table_factor(&mut join.relation)?;
            match constraint {
                JoinConstraint::On(e) => self.expr(e)?,
                JoinConstraint::Using(names) => {
                    for n in names.iter_mut() {
                        column_path(n)?;
                    }
                }
                JoinConstraint::None => {}
                JoinConstraint::Natural => return Err(err("NATURAL JOIN isn't supported")),
            }
        }
        Ok(())
    }

    fn table_factor(&mut self, f: &mut TableFactor) -> Checked {
        match f {
            TableFactor::Table { name, alias, args, with_hints, version, with_ordinality, partitions, json_path, sample, index_hints } => {
                if args.is_some() {
                    return Err(err(format!("Table functions aren't allowed ({name}); query the tables listed under Data")));
                }
                if sample.is_some() {
                    return Err(err("SAMPLE isn't supported"));
                }
                if !with_hints.is_empty() || version.is_some() || *with_ordinality || !partitions.is_empty() || json_path.is_some() || !index_hints.is_empty() {
                    return Err(err("Table hints aren't supported"));
                }
                table_alias(alias)?;
                self.table(name, alias)
            }
            TableFactor::Derived { lateral, subquery, alias, sample } => {
                if *lateral || sample.is_some() {
                    return Err(err("LATERAL and SAMPLE aren't supported"));
                }
                table_alias(alias)?;
                self.query(subquery)
            }
            TableFactor::NestedJoin { table_with_joins, alias } => {
                table_alias(alias)?;
                self.table_with_joins(table_with_joins)
            }
            _ => Err(err("Table functions aren't allowed; query the tables listed under Data")),
        }
    }

    /// Resolves a table name to a CTE in scope or a virtual table; anything else is refused.
    fn table(&mut self, name: &mut ObjectName, alias: &mut Option<TableAlias>) -> Checked {
        let shown = name.to_string();
        let mut parts = Vec::new();
        for part in &name.0 {
            match part {
                ObjectNamePart::Identifier(id) => parts.push(id.clone()),
                _ => return Err(err(format!("Unknown table {shown}"))),
            }
        }
        let [mut id] = <[Ident; 1]>::try_from(parts).map_err(|_| err(format!("Unknown table {shown}: query the tables listed under Data")))?;
        ident(&mut id).map_err(|_| err(format!("Unknown table {shown}")))?;
        let resolved = if let Some(generated) = self.scopes.iter().rev().flat_map(|s| s.iter().rev()).find(|(n, _)| *n == id.value).map(|(_, g)| g.clone()) {
            Ident::with_quote('`', generated)
        } else if let Some(table) = self.tables.iter().find(|t| t.eq_ignore_ascii_case(&id.value)) {
            let index = match self.used.iter().position(|u| u == table) {
                Some(i) => i,
                None => {
                    self.used.push(table.to_string());
                    self.used.len() - 1
                }
            };
            Ident::new(format!("{MARK}tbl_{index}__"))
        } else {
            return Err(err(format!("Unknown table {shown}: query the tables listed under Data")));
        };
        *name = ObjectName::from(vec![resolved]);
        match alias {
            Some(a) => a.explicit = true,
            None => *alias = Some(TableAlias { explicit: true, name: id, columns: Vec::new(), at: None }),
        }
        Ok(())
    }

    fn exprs(&mut self, exprs: &mut [Expr]) -> Checked {
        exprs.iter_mut().try_for_each(|e| self.expr(e))
    }

    fn expr(&mut self, e: &mut Expr) -> Checked {
        match e {
            Expr::Identifier(id) => ident(id),
            Expr::CompoundIdentifier(ids) => {
                if ids.len() > 3 {
                    return Err(err("Column names have at most three parts"));
                }
                ids.iter_mut().try_for_each(ident)
            }
            Expr::CompoundFieldAccess { root, access_chain } => {
                self.expr(root)?;
                for a in access_chain.iter_mut() {
                    match a {
                        AccessExpr::Dot(e) => self.expr(e)?,
                        AccessExpr::Subscript(Subscript::Index { index }) => self.expr(index)?,
                        AccessExpr::Subscript(_) => return Err(err("Array slices aren't supported; use arraySlice()")),
                    }
                }
                Ok(())
            }
            Expr::IsNull(x) | Expr::IsNotNull(x) | Expr::Nested(x) => self.expr(x),
            Expr::InList { expr, list, negated: _ } => {
                // ClickHouse reads `x IN (name)` as a table: IN lists hold values only.
                if !list.iter().all(is_constant) {
                    return Err(err("IN lists take values; use IN (SELECT …) to compare with another table"));
                }
                self.expr(expr)?;
                self.exprs(list)
            }
            Expr::InSubquery { expr, subquery, negated: _ } => {
                self.expr(expr)?;
                self.query(subquery)
            }
            Expr::Between { expr, negated: _, low, high } => {
                self.expr(expr)?;
                self.expr(low)?;
                self.expr(high)
            }
            Expr::BinaryOp { left, op, right } => {
                use BinaryOperator as B;
                if !matches!(op, B::Plus | B::Minus | B::Multiply | B::Divide | B::Modulo | B::StringConcat | B::Gt | B::Lt | B::GtEq | B::LtEq | B::Eq | B::NotEq | B::And | B::Or) {
                    return Err(err(format!("The {op} operator isn't supported")));
                }
                self.expr(left)?;
                self.expr(right)
            }
            Expr::Like { negated: _, any, expr, pattern, escape_char } | Expr::ILike { negated: _, any, expr, pattern, escape_char } => {
                if *any || escape_char.is_some() {
                    return Err(err("LIKE ANY and ESCAPE aren't supported"));
                }
                self.expr(expr)?;
                self.expr(pattern)
            }
            Expr::UnaryOp { op, expr } => {
                if !matches!(op, UnaryOperator::Not | UnaryOperator::Minus | UnaryOperator::Plus) {
                    return Err(err(format!("The {op} operator isn't supported")));
                }
                // `- -1` would render as `--1`, a comment: keep nested signs apart.
                if matches!(op, UnaryOperator::Minus | UnaryOperator::Plus) && matches!(**expr, Expr::UnaryOp { .. }) {
                    let inner = std::mem::replace(&mut **expr, Expr::value(Value::Null));
                    **expr = Expr::Nested(Box::new(inner));
                }
                self.expr(expr)
            }
            Expr::Cast { kind, expr, data_type, format } => {
                if !matches!(kind, CastKind::Cast | CastKind::DoubleColon) || format.is_some() {
                    return Err(err("Use CAST(x AS Type)"));
                }
                let Some(spelled) = ch_type(data_type) else {
                    return Err(err(format!("Unsupported type {}", short(&data_type.to_string()))));
                };
                // Rendered with ClickHouse's spelling (type names are case-sensitive there).
                *data_type = DataType::Custom(ObjectName::from(vec![Ident::new(spelled)]), Vec::new());
                self.expr(expr)
            }
            Expr::Extract { field, syntax: _, expr } => {
                date_field(field)?;
                self.expr(expr)
            }
            Expr::Ceil { expr, field } | Expr::Floor { expr, field } => {
                match field {
                    CeilFloorKind::DateTimeField(DateTimeField::NoDateTime) => {}
                    CeilFloorKind::Scale(v) => self.value(v)?,
                    _ => return Err(err("Use ceil(x) / floor(x)")),
                }
                self.expr(expr)
            }
            Expr::Position { expr, r#in } => {
                self.expr(expr)?;
                self.expr(r#in)
            }
            Expr::Substring { expr, substring_from, substring_for, special: _, shorthand: _ } => {
                self.expr(expr)?;
                for e in [substring_from, substring_for].into_iter().flatten() {
                    self.expr(e)?;
                }
                Ok(())
            }
            Expr::Trim { trim_where: _, trim_what, expr, trim_characters } => {
                if trim_characters.is_some() {
                    return Err(err("Use trim(x)"));
                }
                if let Some(w) = trim_what {
                    self.expr(w)?;
                }
                self.expr(expr)
            }
            Expr::Value(v) => self.value(v),
            Expr::Function(f) => self.function(f),
            Expr::Case { case_token: _, end_token: _, operand, conditions, else_result } => {
                if let Some(o) = operand {
                    self.expr(o)?;
                }
                for c in conditions.iter_mut() {
                    self.expr(&mut c.condition)?;
                    self.expr(&mut c.result)?;
                }
                if let Some(e) = else_result {
                    self.expr(e)?;
                }
                Ok(())
            }
            Expr::Exists { subquery, negated: _ } => self.query(subquery),
            Expr::Subquery(q) => self.query(q),
            Expr::Tuple(xs) => self.exprs(xs),
            Expr::Array(a) => self.exprs(&mut a.elem),
            Expr::Interval(Interval { value, leading_field, leading_precision, last_field, fractional_seconds_precision }) => {
                if leading_precision.is_some() || last_field.is_some() || fractional_seconds_precision.is_some() {
                    return Err(err("Write intervals as INTERVAL 5 MINUTE"));
                }
                if let Some(f) = leading_field {
                    date_field(f)?;
                }
                self.expr(value)
            }
            Expr::Lambda(LambdaFunction { params, body, syntax }) => {
                if *syntax != LambdaSyntax::Arrow {
                    return Err(err("Write lambdas as x -> expression"));
                }
                let params: Vec<&mut LambdaFunctionParameter> = match params {
                    OneOrManyWithParens::One(p) => vec![p],
                    OneOrManyWithParens::Many(ps) => ps.iter_mut().collect(),
                };
                for p in params {
                    if p.data_type.is_some() {
                        return Err(err("Lambda parameters have no type"));
                    }
                    ident(&mut p.name)?;
                }
                self.expr(body)
            }
            Expr::TypedString(_) => Err(err("Typed literals aren't supported; use toDate('…') or toDateTime('…')")),
            other => Err(err(format!("Unsupported expression: {}", short(&other.to_string())))),
        }
    }

    fn value(&mut self, v: &mut ValueWithSpan) -> Checked {
        match &v.value {
            Value::SingleQuotedString(s) => {
                let s = s.clone();
                v.value = Value::Placeholder(format!("{MARK}lit_{}__", self.literals.len()));
                self.literals.push(s);
                Ok(())
            }
            Value::Number(n, false) if is_number(n) => Ok(()),
            Value::Boolean(_) | Value::Null => Ok(()),
            Value::Placeholder(p) => Err(err(format!("Parameters ({p}) aren't supported; write the value in the query"))),
            _ => Err(err("Unsupported literal; write strings in single quotes")),
        }
    }

    fn function(&mut self, f: &mut Function) -> Checked {
        let Function { name, uses_odbc_syntax, parameters, args, within_group, filter, null_treatment, over } = f;
        let shown = name.to_string();
        let allowed = match &name.0[..] {
            [ObjectNamePart::Identifier(id)] => id.quote_style.is_none() && ALLOWED_FUNCTIONS.iter().any(|a| a.eq_ignore_ascii_case(&id.value)),
            _ => false,
        };
        if !allowed {
            return Err(err(format!("The function {} isn't available; see Docs for the functions you can use", short(&shown))));
        }
        if *uses_odbc_syntax || !within_group.is_empty() || filter.is_some() || null_treatment.is_some() {
            return Err(err(format!("{shown}: FILTER, WITHIN GROUP and NULLS options aren't supported")));
        }
        self.function_args(parameters)?;
        self.function_args(args)?;
        match over {
            None => Ok(()),
            Some(WindowType::NamedWindow(id)) => ident(id),
            Some(WindowType::WindowSpec(spec)) => self.window(spec),
        }
    }

    fn function_args(&mut self, args: &mut FunctionArguments) -> Checked {
        match args {
            FunctionArguments::None => Ok(()),
            FunctionArguments::Subquery(_) => Err(err("Pass subqueries to functions in parentheses")),
            FunctionArguments::List(FunctionArgumentList { duplicate_treatment: _, args, clauses }) => {
                if !clauses.is_empty() {
                    return Err(err("Clauses inside function calls aren't supported"));
                }
                for a in args.iter_mut() {
                    match a {
                        FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) => self.expr(e)?,
                        FunctionArg::Unnamed(FunctionArgExpr::Wildcard) => {}
                        FunctionArg::Unnamed(FunctionArgExpr::QualifiedWildcard(n)) => column_path(n)?,
                        _ => return Err(err("Named function arguments aren't supported")),
                    }
                }
                Ok(())
            }
        }
    }

    fn window(&mut self, spec: &mut WindowSpec) -> Checked {
        let WindowSpec { window_name, partition_by, order_by, window_frame } = spec;
        if let Some(n) = window_name {
            ident(n)?;
        }
        self.exprs(partition_by)?;
        for o in order_by.iter_mut() {
            self.order_by_expr(o)?;
        }
        if let Some(frame) = window_frame {
            for bound in std::iter::once(&mut frame.start_bound).chain(frame.end_bound.as_mut()) {
                if let WindowFrameBound::Preceding(Some(e)) | WindowFrameBound::Following(Some(e)) = bound {
                    self.expr(e)?;
                }
            }
        }
        Ok(())
    }

    fn order_by(&mut self, o: &mut OrderBy) -> Checked {
        if o.interpolate.is_some() {
            return Err(err("INTERPOLATE isn't supported"));
        }
        match &mut o.kind {
            OrderByKind::All(opts) => order_options(opts),
            OrderByKind::Expressions(xs) => xs.iter_mut().try_for_each(|x| self.order_by_expr(x)),
        }
    }

    fn order_by_expr(&mut self, o: &mut OrderByExpr) -> Checked {
        order_options(&o.options)?;
        if let Some(fill) = &mut o.with_fill {
            for e in [&mut fill.from, &mut fill.to, &mut fill.step].into_iter().flatten() {
                self.expr(e)?;
            }
        }
        self.expr(&mut o.expr)
    }

    fn limit(&mut self, l: &mut LimitClause) -> Checked {
        match l {
            LimitClause::LimitOffset { limit, offset, limit_by } => {
                if let Some(e) = limit {
                    self.expr(e)?;
                }
                if let Some(o) = offset {
                    self.expr(&mut o.value)?;
                }
                self.exprs(limit_by)
            }
            LimitClause::OffsetCommaLimit { offset, limit } => {
                self.expr(offset)?;
                self.expr(limit)
            }
        }
    }
}

/// Identifiers are plain ASCII names, always rendered in backticks.
fn ident(id: &mut Ident) -> Checked {
    if !matches!(id.quote_style, None | Some('`') | Some('"')) {
        return Err(err(format!("Unsupported identifier {id}")));
    }
    let v = &id.value;
    let valid = !v.is_empty()
        && v.len() <= MAX_IDENT_CHARS
        && v.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !v.to_ascii_lowercase().contains("__oc");
    if !valid {
        return Err(err(format!("Invalid name {}: names use letters, digits and _", short(v))));
    }
    id.quote_style = Some('`');
    Ok(())
}

/// `alias.*`, `USING (col)`: one or two plain names.
fn column_path(name: &mut ObjectName) -> Checked {
    if name.0.is_empty() || name.0.len() > 2 {
        return Err(err(format!("Invalid name {name}")));
    }
    for part in name.0.iter_mut() {
        match part {
            ObjectNamePart::Identifier(id) => ident(id)?,
            _ => return Err(err("Invalid name")),
        }
    }
    Ok(())
}

fn table_alias(alias: &mut Option<TableAlias>) -> Checked {
    match alias {
        None => Ok(()),
        Some(a) if a.columns.is_empty() && a.at.is_none() => ident(&mut a.name),
        Some(_) => Err(err("Column aliases on tables aren't supported")),
    }
}

fn wildcard_options(opts: &mut WildcardAdditionalOptions) -> Checked {
    let WildcardAdditionalOptions { wildcard_token: _, opt_ilike, opt_exclude, opt_except, opt_replace, opt_rename, opt_alias } = opts;
    if opt_ilike.is_some() || opt_exclude.is_some() || opt_replace.is_some() || opt_rename.is_some() || opt_alias.is_some() {
        return Err(err("Only * EXCEPT (…) is supported after *"));
    }
    if let Some(except) = opt_except {
        ident(&mut except.first_element)?;
        except.additional_elements.iter_mut().try_for_each(ident)?;
    }
    Ok(())
}

fn group_modifiers(mods: &[GroupByWithModifier]) -> Checked {
    if mods.iter().all(|m| matches!(m, GroupByWithModifier::Rollup | GroupByWithModifier::Cube | GroupByWithModifier::Totals)) {
        Ok(())
    } else {
        Err(err("GROUPING SETS isn't supported"))
    }
}

fn order_options(o: &OrderByOptions) -> Checked {
    match o.sort {
        Some(OrderBySort::Using(_)) => Err(err("ORDER BY … USING isn't supported")),
        _ => Ok(()),
    }
}

fn date_field(f: &DateTimeField) -> Checked {
    const FIELDS: &[&str] = &["SECOND", "SECONDS", "MINUTE", "MINUTES", "HOUR", "HOURS", "DAY", "DAYS", "WEEK", "WEEKS", "MONTH", "MONTHS", "QUARTER", "YEAR", "YEARS"];
    let shown = f.to_string();
    if FIELDS.contains(&shown.as_str()) {
        Ok(())
    } else {
        Err(err(format!("Unsupported time unit {}", short(&shown))))
    }
}

/// The ClickHouse name of a type a value may be cast to.
fn ch_type(t: &DataType) -> Option<String> {
    let name = match t {
        DataType::Int8(None) => "Int8",
        DataType::Int16 => "Int16",
        DataType::Int32 => "Int32",
        DataType::Int64 => "Int64",
        DataType::Int128 => "Int128",
        DataType::Int256 => "Int256",
        DataType::UInt8 => "UInt8",
        DataType::UInt16 => "UInt16",
        DataType::UInt32 => "UInt32",
        DataType::UInt64 => "UInt64",
        DataType::UInt128 => "UInt128",
        DataType::UInt256 => "UInt256",
        DataType::Float32 => "Float32",
        DataType::Float64 => "Float64",
        DataType::Bool | DataType::Boolean => "Bool",
        DataType::Date => "Date",
        DataType::Date32 => "Date32",
        DataType::Datetime(None) => "DateTime",
        DataType::String(None) => "String",
        DataType::Uuid => "UUID",
        DataType::Datetime64(p, None) if *p <= 9 => return Some(format!("DateTime64({p})")),
        DataType::Nullable(inner) => return Some(format!("Nullable({})", ch_type(inner)?)),
        DataType::LowCardinality(inner) => return Some(format!("LowCardinality({})", ch_type(inner)?)),
        DataType::Array(ArrayElemTypeDef::Parenthesis(inner)) => return Some(format!("Array({})", ch_type(inner)?)),
        _ => return None,
    };
    Some(name.to_string())
}

fn is_number(n: &str) -> bool {
    let (mantissa, exponent) = match n.find(['e', 'E']) {
        Some(i) => (&n[..i], Some(&n[i + 1..])),
        None => (n, None),
    };
    let mut parts = mantissa.splitn(2, '.');
    let int = parts.next().unwrap_or("");
    let frac = parts.next();
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    let mantissa_ok = match frac {
        None => digits(int),
        Some(f) => (int.is_empty() || digits(int)) && (f.is_empty() || digits(f)) && !(int.is_empty() && f.is_empty()),
    };
    let exponent_ok = exponent.is_none_or(|e| digits(e.strip_prefix(['+', '-']).unwrap_or(e)));
    mantissa_ok && exponent_ok
}

/// Literals and tuples of literals (what an IN list may hold).
fn is_constant(e: &Expr) -> bool {
    match e {
        Expr::Value(_) => true,
        Expr::UnaryOp { op: UnaryOperator::Minus | UnaryOperator::Plus, expr } => matches!(**expr, Expr::Value(_)),
        Expr::Tuple(xs) => xs.iter().all(is_constant),
        _ => false,
    }
}

/// Sets the row cap; returns whether the query must be wrapped to apply it (LIMIT … BY, computed limits).
fn cap(q: &mut Query, max_rows: u64) -> bool {
    let capped = || Some(Expr::value(Value::Number((max_rows + 1).to_string(), false)));
    match &mut q.limit_clause {
        None => {
            q.limit_clause = Some(LimitClause::LimitOffset { limit: capped(), offset: None, limit_by: Vec::new() });
            false
        }
        Some(LimitClause::LimitOffset { limit, limit_by, .. }) if limit_by.is_empty() => match limit {
            None => {
                *limit = capped();
                false
            }
            Some(Expr::Value(ValueWithSpan { value: Value::Number(n, false), .. })) => {
                if n.parse::<u64>().map_or(true, |n| n > max_rows) {
                    *limit = capped();
                }
                false
            }
            Some(_) => true,
        },
        Some(_) => true,
    }
}

/// Cuts the rendered text at the markers. The text left is checked once more against what a
/// rendered query can contain: any quote, comment or parameter syntax means something slipped
/// through, and the query is refused.
fn split(text: &str, literals: &[String], tables: &[String]) -> Result<Vec<SqlPiece>, SqlError> {
    let mut pieces = Vec::new();
    let mut rest = text;
    let mut plain = String::new();
    while let Some(at) = rest.find(MARK) {
        let after = &rest[at + MARK.len()..];
        let kind = if after.starts_with("lit_") {
            Some(true)
        } else if after.starts_with("tbl_") {
            Some(false)
        } else {
            None
        };
        let Some(is_literal) = kind else {
            // Generated CTE names stay in the text.
            plain.push_str(&rest[..at + MARK.len()]);
            rest = after;
            continue;
        };
        let digits: String = after[4..].chars().take_while(char::is_ascii_digit).collect();
        let end = 4 + digits.len();
        let index: usize = digits.parse().map_err(|_| unsafe_query())?;
        if !after[end..].starts_with("__") {
            return Err(unsafe_query());
        }
        plain.push_str(&rest[..at]);
        if !plain.is_empty() {
            pieces.push(SqlPiece::Text(std::mem::take(&mut plain)));
        }
        pieces.push(if is_literal { SqlPiece::Literal(literals.get(index).ok_or_else(unsafe_query)?.clone()) } else { SqlPiece::Table(tables.get(index).ok_or_else(unsafe_query)?.clone()) });
        rest = &after[end + 2..];
    }
    plain.push_str(rest);
    if !plain.is_empty() {
        pieces.push(SqlPiece::Text(plain));
    }
    for p in &pieces {
        if let SqlPiece::Text(t) = p {
            let chars_ok = t.chars().all(|c| c.is_ascii_alphanumeric() || " _`(),.*+-/%=<>![]|:".contains(c));
            if !chars_ok || t.contains("--") || t.contains("/*") || t.contains("*/") {
                return Err(unsafe_query());
            }
        }
    }
    Ok(pieces)
}

fn unsafe_query() -> SqlError {
    err("This query can't be run as written; simplify it")
}

fn short(s: &str) -> String {
    let mut out: String = s.chars().take(80).collect();
    if s.chars().count() > 80 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLES: &[&str] = &["hosts", "spans", "logs", "monitors", "containers"];

    /// Renders pieces with `?` for literals and `<name>` for tables; returns the literals too.
    fn render(sql: &str) -> (String, Vec<String>) {
        let checked = check_sql(sql, TABLES, 10_000).unwrap_or_else(|e| panic!("{sql}: {e}"));
        let mut out = String::new();
        let mut lits = Vec::new();
        for p in &checked.pieces {
            match p {
                SqlPiece::Text(t) => out.push_str(t),
                SqlPiece::Literal(l) => {
                    out.push('?');
                    lits.push(l.clone());
                }
                SqlPiece::Table(t) => out.push_str(&format!("<{t}>")),
            }
        }
        (out, lits)
    }

    fn refused(sql: &str) -> String {
        match check_sql(sql, TABLES, 10_000) {
            Ok(c) => panic!("accepted: {sql}\n{:?}", c.pieces),
            Err(e) => e.0,
        }
    }

    #[test]
    fn virtual_tables_become_placeholders_with_their_name_as_alias() {
        let (sql, lits) = render("SELECT name, os FROM hosts");
        assert_eq!(sql, "SELECT `name`, `os` FROM <hosts> AS `hosts` LIMIT 10001");
        assert!(lits.is_empty());
        let (sql, _) = render("select h.name from hosts h join containers as c on c.host = h.name");
        assert_eq!(sql, "SELECT `h`.`name` FROM <hosts> AS `h` JOIN <containers> AS `c` ON `c`.`host` = `h`.`name` LIMIT 10001");
        assert_eq!(check_sql("SELECT 1 FROM hosts, hosts AS b, spans", TABLES, 100).unwrap().tables, vec!["hosts", "spans"]);
    }

    #[test]
    fn table_names_match_case_insensitively_and_quoted() {
        let (sql, _) = render("SELECT * FROM `HOSTS`");
        assert_eq!(sql, "SELECT * FROM <hosts> AS `HOSTS` LIMIT 10001");
        let (sql, _) = render("SELECT * FROM \"hosts\"");
        assert_eq!(sql, "SELECT * FROM <hosts> AS `hosts` LIMIT 10001");
    }

    #[test]
    fn string_literals_are_bound_never_rendered() {
        let (sql, lits) = render("SELECT * FROM hosts WHERE os = 'linux' AND name LIKE 'i-%'");
        assert_eq!(sql, "SELECT * FROM <hosts> AS `hosts` WHERE `os` = ? AND `name` LIKE ? LIMIT 10001");
        assert_eq!(lits, vec!["linux", "i-%"]);
        let (_, lits) = render("SELECT 'it''s' FROM hosts");
        assert_eq!(lits, vec!["it's"]);
    }

    #[test]
    fn backslashes_in_literals_cannot_break_out_of_the_string() {
        // ClickHouse reads `\'` as an escaped quote: a literal ending in a backslash must never be
        // rendered back into SQL text, or the rest of the query would change meaning.
        for sql in ["SELECT 'x\\' , ' UNION ALL SELECT * FROM system.tables --' FROM hosts", "SELECT 'a\\\\' FROM hosts", "SELECT * FROM hosts WHERE name = '\\' OR 1=1 --'"] {
            if let Ok(c) = check_sql(sql, TABLES, 100) {
                for p in &c.pieces {
                    if let SqlPiece::Text(t) = p {
                        assert!(!t.contains('\'') && !t.contains('\\') && !t.contains("system"), "{sql} rendered {t}");
                    }
                }
            }
        }
    }

    #[test]
    fn only_a_single_select_is_accepted() {
        for sql in [
            "INSERT INTO hosts VALUES (1)",
            "DROP TABLE hosts",
            "CREATE TABLE x (a Int32) ENGINE = Memory",
            "ALTER TABLE hosts DELETE WHERE 1",
            "TRUNCATE TABLE hosts",
            "DELETE FROM hosts WHERE 1",
            "UPDATE hosts SET name = 'x'",
            "SELECT 1; SELECT 2",
            "SELECT * FROM hosts; DROP TABLE hosts",
            "SET max_memory_usage = 0",
            "SHOW TABLES",
            "DESCRIBE TABLE hosts",
            "EXPLAIN SELECT * FROM hosts",
            "GRANT ALL ON *.* TO default",
            "KILL QUERY WHERE 1",
            "OPTIMIZE TABLE hosts",
            "",
            "   ",
        ] {
            refused(sql);
        }
        // One trailing semicolon is fine.
        render("SELECT * FROM hosts;");
    }

    #[test]
    fn unknown_and_raw_tables_are_refused() {
        for sql in [
            "SELECT * FROM otel.otel_traces",
            "SELECT * FROM otel_traces",
            "SELECT * FROM oc.logs",
            "SELECT * FROM logs_raw",
            "SELECT * FROM system.tables",
            "SELECT * FROM system.query_log",
            "SELECT * FROM \"otel\".\"otel_traces\"",
            "SELECT * FROM `otel`.`otel_traces`",
            "SELECT * FROM default.hosts",
            "SELECT * FROM information_schema.tables",
            "SELECT * FROM postgres.monitors",
        ] {
            refused(sql);
        }
    }

    #[test]
    fn table_functions_are_refused() {
        for sql in [
            "SELECT * FROM remote('127.0.0.1', otel.otel_traces)",
            "SELECT * FROM remoteSecure('ch:9440', 'otel', 'otel_traces')",
            "SELECT * FROM cluster('oc', otel.otel_traces)",
            "SELECT * FROM url('http://169.254.169.254/latest/meta-data', CSV)",
            "SELECT * FROM file('/etc/passwd', 'LineAsString')",
            "SELECT * FROM s3('http://rustfs:9000/archives/x.parquet')",
            "SELECT * FROM numbers(1000000000000)",
            "SELECT * FROM mysql('host:3306', 'db', 't', 'u', 'p')",
            "SELECT * FROM postgresql('postgres:5432', 'oc', 'monitors', 'oc', 'oc')",
            "SELECT * FROM merge('otel', '.*')",
            "SELECT * FROM executable('x.sh', TabSeparated, 'a String')",
            "SELECT * FROM format(JSONEachRow, '{}')",
            "SELECT * FROM input('a String')",
            "SELECT * FROM hosts(1)",
            "SELECT * FROM TABLE(hosts)",
            "SELECT * FROM UNNEST([1, 2])",
        ] {
            refused(sql);
        }
    }

    #[test]
    fn functions_outside_the_allowlist_are_refused() {
        for sql in [
            "SELECT file('/etc/passwd') FROM hosts",
            "SELECT dictGet('d', 'v', 1) FROM hosts",
            "SELECT joinGet('otel.otel_traces', 'x', 1) FROM hosts",
            "SELECT currentUser()",
            "SELECT hostName()",
            "SELECT getSetting('max_memory_usage')",
            "SELECT hasColumnInTable('otel', 'otel_traces', 'TraceId')",
            "SELECT url('http://x')",
            "SELECT sleep(3)",
            "SELECT throwIf(1)",
            "SELECT otel.count() FROM hosts",
            "SELECT addressToSymbol(1)",
        ] {
            refused(sql);
        }
    }

    #[test]
    fn subqueries_cannot_reach_raw_tables() {
        for sql in [
            "SELECT * FROM hosts WHERE name IN (SELECT host FROM otel.otel_traces)",
            "SELECT * FROM hosts WHERE EXISTS (SELECT 1 FROM oc.logs)",
            "SELECT (SELECT count() FROM system.tables) FROM hosts",
            "SELECT * FROM (SELECT * FROM otel.otel_traces) AS t",
            "SELECT * FROM hosts UNION ALL SELECT * FROM otel.otel_logs",
            "SELECT * FROM hosts UNION ALL (SELECT * FROM oc.logs)",
            "WITH x AS (SELECT * FROM oc.logs) SELECT * FROM x",
            "SELECT * FROM hosts JOIN otel.otel_traces ON 1 = 1",
            "SELECT * FROM hosts WHERE name IN (otel_traces)",
            "SELECT * FROM hosts WHERE name IN ((otel_traces))",
            "SELECT * FROM hosts WHERE name IN (otel.otel_traces)",
            "SELECT * FROM hosts WHERE name IN (1, otel_traces)",
        ] {
            refused(sql);
        }
    }

    #[test]
    fn subqueries_over_virtual_tables_are_rewritten() {
        let (sql, _) = render("SELECT * FROM hosts WHERE name IN (SELECT host FROM spans) AND EXISTS (SELECT 1 FROM logs)");
        assert_eq!(sql, "SELECT * FROM <hosts> AS `hosts` WHERE `name` IN (SELECT `host` FROM <spans> AS `spans`) AND EXISTS (SELECT 1 FROM <logs> AS `logs`) LIMIT 10001");
        let (sql, _) = render("SELECT name FROM hosts UNION ALL SELECT name FROM containers");
        assert_eq!(sql, "SELECT `name` FROM <hosts> AS `hosts` UNION ALL SELECT `name` FROM <containers> AS `containers` LIMIT 10001");
    }

    #[test]
    fn query_clauses_that_change_execution_are_refused() {
        for sql in [
            "SELECT * FROM hosts SETTINGS max_memory_usage = 0",
            "SELECT * FROM hosts SETTINGS readonly = 0",
            "SELECT * FROM hosts FORMAT TabSeparated",
            "SELECT * FROM hosts INTO OUTFILE '/tmp/x'",
            "SELECT * INTO new_table FROM hosts",
            "SELECT * FROM hosts FOR UPDATE",
            "SELECT * FROM hosts SAMPLE 0.1",
            "SELECT * FROM hosts GLOBAL JOIN spans ON 1 = 1",
            "SELECT * FROM hosts ARRAY JOIN tags",
            "SELECT * FROM hosts AS h (a, b)",
            "SELECT TOP 5 * FROM hosts",
        ] {
            refused(sql);
        }
    }

    #[test]
    fn comments_are_dropped_and_cannot_hide_tables() {
        let (sql, _) = render("SELECT name -- the host\nFROM hosts /* all of them */ -- ; DROP TABLE hosts");
        assert_eq!(sql, "SELECT `name` FROM <hosts> AS `hosts` LIMIT 10001");
        refused("SELECT * FROM /* hosts */ otel.otel_traces");
        refused("SELECT * FROM hosts /*! UNION ALL SELECT * FROM oc.logs */ UNION ALL SELECT * FROM oc.logs");
    }

    #[test]
    fn unicode_and_control_character_tricks_are_refused() {
        for sql in [
            "SELECT * FROM h\u{043e}sts",    // Cyrillic o
            "SELECT * FROM ho\u{200b}sts",   // zero-width space
            "SELECT * FROM \u{ff48}osts",    // fullwidth h
            "SELECT * FROM hosts\u{0000}",   // NUL
            "SELECT * FROM `ho\u{202e}sts`", // right-to-left override
            "SELECT n\u{0430}me FROM hosts", // Cyrillic a in a column
        ] {
            refused(sql);
        }
        // Non-ASCII text inside string literals is data, bound as a parameter.
        let (_, lits) = render("SELECT * FROM hosts WHERE name = 'h\u{043e}st \u{1F600}'");
        assert_eq!(lits, vec!["h\u{043e}st \u{1F600}"]);
    }

    #[test]
    fn placeholders_and_reserved_names_are_refused() {
        for sql in [
            "SELECT * FROM hosts WHERE name = ?",
            "SELECT * FROM hosts WHERE name = $1",
            "SELECT * FROM hosts WHERE name = {oc_org:String}",
            "SELECT * FROM hosts WHERE name = :org",
            "SELECT __oc_tbl_0__ FROM hosts",
            "SELECT * FROM hosts AS __oc_lit_0__",
            "WITH __oc_cte_0 AS (SELECT 1) SELECT * FROM __oc_cte_0",
        ] {
            refused(sql);
        }
    }

    #[test]
    fn ctes_are_renamed_so_out_of_scope_names_never_reach_real_tables() {
        let (sql, _) = render("WITH busy AS (SELECT name FROM hosts) SELECT * FROM busy");
        assert_eq!(sql, "WITH `__oc_cte_0` AS (SELECT `name` FROM <hosts> AS `hosts`) SELECT * FROM `__oc_cte_0` AS `busy` LIMIT 10001");
        // ClickHouse attaches WITH to the first SELECT of a UNION; renaming keeps the second branch from
        // reading a real table with the CTE's name.
        let (sql, _) = render("WITH otel_traces AS (SELECT 1 AS a) SELECT a FROM otel_traces UNION ALL SELECT a FROM otel_traces");
        assert!(!sql.contains("FROM `otel_traces`") && !sql.contains("FROM otel_traces"), "{sql}");
        // A CTE of a subquery isn't visible outside it.
        refused("SELECT * FROM (WITH x AS (SELECT 1) SELECT * FROM x) AS t, x");
        // A CTE can't read itself, and recursive CTEs are refused.
        refused("WITH x AS (SELECT * FROM x) SELECT * FROM x");
        refused("WITH RECURSIVE x AS (SELECT 1 UNION ALL SELECT 1 FROM x) SELECT * FROM x");
        // A CTE named like a virtual table shadows it inside the query.
        let (sql, _) = render("WITH hosts AS (SELECT name FROM hosts) SELECT * FROM hosts");
        assert_eq!(sql, "WITH `__oc_cte_0` AS (SELECT `name` FROM <hosts> AS `hosts`) SELECT * FROM `__oc_cte_0` AS `hosts` LIMIT 10001");
    }

    #[test]
    fn results_are_capped() {
        assert!(render("SELECT * FROM hosts").0.ends_with(" LIMIT 10001"));
        assert!(render("SELECT * FROM hosts LIMIT 50").0.ends_with(" LIMIT 50"));
        assert!(render("SELECT * FROM hosts LIMIT 50 OFFSET 10").0.ends_with(" LIMIT 50 OFFSET 10"));
        assert!(render("SELECT * FROM hosts LIMIT 999999").0.ends_with(" LIMIT 10001"));
        // LIMIT n BY limits per group; the whole result is capped around it.
        let (sql, _) = render("SELECT name, os FROM hosts ORDER BY name LIMIT 1 BY os");
        assert_eq!(sql, "SELECT * FROM (SELECT `name`, `os` FROM <hosts> AS `hosts` ORDER BY `name` LIMIT 1 BY `os`) LIMIT 10001");
    }

    #[test]
    fn clickhouse_expressions_are_accepted() {
        let (sql, lits) = render(
            "SELECT service, quantile(0.95)(duration_ms) AS p95, countIf(status = 'error') AS errors, \
             toStartOfInterval(timestamp, INTERVAL 5 MINUTE) AS t, tags['env'] AS env, arrayMap(x -> x * 2, [1, 2]) AS doubled, \
             row_number() OVER (PARTITION BY service ORDER BY p95 DESC) AS rank, CAST(errors AS Float64) / greatest(count(), 1) AS rate, \
             CASE WHEN p95 > 1000 THEN 'slow' ELSE 'ok' END AS speed, count(DISTINCT host) AS hosts, \
             if(isNull(env), 'none', env) AS e, -1 AS neg, NOT 1 = 2 AS t2 \
             FROM spans WHERE timestamp >= now() - INTERVAL 1 HOUR AND name NOT IN ('a', 'b') AND duration_ms BETWEEN 1 AND 2 \
             GROUP BY service, t, env HAVING count() > 10 ORDER BY p95 DESC NULLS LAST LIMIT 20",
        );
        assert!(sql.contains("quantile(0.95)(`duration_ms`)"), "{sql}");
        assert!(sql.contains("`tags`[?]"), "{sql}");
        assert_eq!(lits, vec!["error", "env", "slow", "ok", "none", "a", "b"]);
    }

    #[test]
    fn rendering_never_produces_comment_or_quote_syntax() {
        // `- -1` must not come out as `--1` (a ClickHouse comment that would swallow the rest).
        let (sql, _) = render("SELECT - -1, - - -2 FROM hosts");
        assert!(!sql.contains("--"), "{sql}");
        refused("SELECT CAST(name AS Enum8('a' = 1)) FROM hosts");
        refused("SELECT * FROM hosts WHERE name = 0x41");
        refused("SELECT * FROM hosts WHERE name = E'\\x41'");
        refused("SELECT * FROM hosts WHERE name = N'x'");
        refused("SELECT * FROM hosts WHERE created = DATE '2024-01-01'");
        let (sql, lits) = render("SELECT * FROM hosts WHERE (name, os) IN (('a', 'linux'), ('b', 'windows')) AND cpu::Float64 > 1.5e2");
        assert!(sql.contains("IN ((?, ?), (?, ?))") && sql.contains("`cpu`::Float64 > 1.5e2"), "{sql}");
        let (sql, _) = render("SELECT CAST(cpu AS Nullable(Int64)), CAST(name AS String), CAST(t AS DateTime64(3)), CAST(a AS Array(UInt8)) FROM hosts");
        assert!(sql.contains("CAST(`cpu` AS Nullable(Int64)), CAST(`name` AS String), CAST(`t` AS DateTime64(3)), CAST(`a` AS Array(UInt8))"), "{sql}");
        assert_eq!(lits.len(), 4);
        // Computed limits are applied around the query.
        assert!(render("SELECT * FROM hosts LIMIT 10 + 5").0.starts_with("SELECT * FROM (SELECT"));
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_crash() {
        let sql = format!("SELECT {}1{} FROM hosts", "(".repeat(5000), ")".repeat(5000));
        refused(&sql);
        refused(&"SELECT 1 FROM hosts WHERE 1 = 1".repeat(20_000));
    }

    #[test]
    fn errors_say_what_is_wrong() {
        assert!(refused("SELECT * FROM otel.otel_traces").contains("otel.otel_traces"));
        assert!(refused("SELECT * FROM nope").contains("Unknown table nope"));
        assert!(refused("SELECT file('x') FROM hosts").contains("file"));
        assert!(refused("DROP TABLE hosts").contains("Only SELECT"));
    }
}
