/// Escapes a value for a PowerShell single-quoted string literal.
///
/// PowerShell closes a single-quoted string on every `'`; doubling it is the
/// language's literal-quote escape. `$` is inert inside single quotes.
String psSingleQuote(String value) => value.replaceAll("'", "''");

/// Escapes a value for a PowerShell double-quoted string literal.
///
/// Backtick is the escape character, so it must be doubled first, then `"` and
/// `$` are escaped to keep them literal instead of ending the string or
/// expanding a variable/subexpression.
String psDoubleQuote(String value) =>
    value.replaceAll('`', '``').replaceAll('"', '`"').replaceAll(r'$', r'`$');
