module.exports = grammar({
    name: "kome",

    extras: ($) => [
        /\s/,
        $.comment,
    ],

    word: ($) => $.identifier,

    rules: {
        source_file: ($) => repeat(
            choice(
                $.attribute,
                $.string,
                $.percentage,
                $.number,
                $.boolean,
                $.null,
                $.keyword,
                $.identifier,
                $.operator,
                $.punctuation,
            ),
        ),

        comment: () => token(
            seq(
                "//",
                /[^\n\r]*/,
            ),
        ),

        attribute: ($) => seq(
            "@",
            field("name", $.identifier),
        ),

        string: ($) => seq(
            "\"",
            repeat(
                choice(
                    $.escape_sequence,
                    $.string_content,
                ),
            ),
            "\"",
        ),

        string_content: () => token.immediate(
            /[^"\\\n\r]+/,
        ),

        escape_sequence: () => token.immediate(
            seq(
                "\\",
                /["\\nrt0{}]/,
            ),
        ),

        percentage: () => token(
            prec(
                2,
                /[0-9]+(\.[0-9]+)?%/,
            ),
        ),

        number: () => token(
            /[0-9]+(\.[0-9]+)?/,
        ),

        boolean: () => choice(
            "true",
            "false",
        ),

        null: () => "null",

        keyword: () => choice(
            "fn",
            "component",
            "enum",
            "extension",
            "recipe",
            "state",
            "let",
            "mut",
            "use",
            "if",
            "else",
            "while",
            "for",
            "in",
            "return",
            "break",
            "continue",
            "is",
        ),

        identifier: () => token(
            /[A-Za-z_][A-Za-z0-9_]*/,
        ),

        operator: () => choice(
            "->",
            "=>",
            "+=",
            "-=",
            "*=",
            "/=",
            "==",
            "!=",
            "<=",
            ">=",
            "&&",
            "||",
            "=",
            "+",
            "-",
            "*",
            "/",
            "!",
            "<",
            ">",
            "?",
            "|",
        ),

        punctuation: () => choice(
            "(",
            ")",
            "{",
            "}",
            "[",
            "]",
            ",",
            ".",
            ":",
        ),
    },
});