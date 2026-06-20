package org.mochios.kome.lang

import com.intellij.lexer.LexerBase
import com.intellij.psi.TokenType
import com.intellij.psi.tree.IElementType

class KomeLexer : LexerBase() {
    private var source: CharSequence = ""
    private var sourceEnd = 0

    private var currentStart = 0
    private var currentEnd = 0
    private var currentType: IElementType? = null

    override fun start(
        buffer: CharSequence,
        startOffset: Int,
        endOffset: Int,
        initialState: Int,
    ) {
        source = buffer
        sourceEnd = endOffset
        currentStart = startOffset
        currentEnd = startOffset

        locateToken()
    }

    override fun getState(): Int {
        return 0
    }

    override fun getTokenType(): IElementType? {
        return currentType
    }

    override fun getTokenStart(): Int {
        return currentStart
    }

    override fun getTokenEnd(): Int {
        return currentEnd
    }

    override fun advance() {
        currentStart = currentEnd
        locateToken()
    }

    override fun getBufferSequence(): CharSequence {
        return source
    }

    override fun getBufferEnd(): Int {
        return sourceEnd
    }

    private fun locateToken() {
        if (currentStart >= sourceEnd) {
            currentEnd = currentStart
            currentType = null
            return
        }

        val character = source[currentStart]

        when {
            character.isWhitespace() -> {
                lexWhitespace()
            }

            startsWith(currentStart, "//") -> {
                lexLineComment()
            }

            character == '"' -> {
                lexString()
            }

            character == '@' -> {
                lexAttribute()
            }

            character in '0'..'9' -> {
                lexNumber()
            }

            isIdentifierStart(character) -> {
                lexIdentifier()
            }

            else -> {
                lexSymbol()
            }
        }
    }

    private fun lexWhitespace() {
        var index = currentStart + 1

        while (
            index < sourceEnd &&
            source[index].isWhitespace()
        ) {
            index += 1
        }

        currentEnd = index
        currentType = TokenType.WHITE_SPACE
    }

    private fun lexLineComment() {
        var index = currentStart + 2

        while (index < sourceEnd) {
            val character = source[index]

            if (
                character == '\n' ||
                character == '\r'
            ) {
                break
            }

            index += 1
        }

        currentEnd = index
        currentType = KomeTokenTypes.LINE_COMMENT
    }

    private fun lexString() {
        var index = currentStart + 1
        var escaped = false

        while (index < sourceEnd) {
            val character = source[index]

            if (escaped) {
                escaped = false
                index += 1
                continue
            }

            when (character) {
                '\\' -> {
                    escaped = true
                    index += 1
                }

                '"' -> {
                    index += 1
                    break
                }

                '\n', '\r' -> {
                    break
                }

                else -> {
                    index += 1
                }
            }
        }

        currentEnd = index
        currentType = KomeTokenTypes.STRING
    }

    private fun lexAttribute() {
        var index = currentStart + 1

        while (
            index < sourceEnd &&
            isIdentifierContinue(source[index])
        ) {
            index += 1
        }

        currentEnd = index
        currentType = KomeTokenTypes.ATTRIBUTE
    }

    private fun lexNumber() {
        var index = currentStart

        while (
            index < sourceEnd &&
            source[index] in '0'..'9'
        ) {
            index += 1
        }

        if (
            index + 1 < sourceEnd &&
            source[index] == '.' &&
            source[index + 1] in '0'..'9'
        ) {
            index += 1

            while (
                index < sourceEnd &&
                source[index] in '0'..'9'
            ) {
                index += 1
            }
        }

        if (
            index < sourceEnd &&
            source[index] == '%'
        ) {
            index += 1
        }

        currentEnd = index
        currentType = KomeTokenTypes.NUMBER
    }

    private fun lexIdentifier() {
        var index = currentStart + 1

        while (
            index < sourceEnd &&
            isIdentifierContinue(source[index])
        ) {
            index += 1
        }

        val identifier = source
            .subSequence(currentStart, index)
            .toString()

        currentEnd = index
        currentType = when {
            identifier in KEYWORDS -> {
                KomeTokenTypes.KEYWORD
            }

            identifier.first().isUpperCase() -> {
                KomeTokenTypes.TYPE_IDENTIFIER
            }

            nextNonWhitespaceCharacter(index) == '(' -> {
                KomeTokenTypes.CALLABLE_IDENTIFIER
            }

            else -> {
                KomeTokenTypes.IDENTIFIER
            }
        }
    }

    private fun lexSymbol() {
        val compoundOperator = if (
            currentStart + 2 <= sourceEnd
        ) {
            source
                .subSequence(
                    currentStart,
                    currentStart + 2,
                )
                .toString()
        } else {
            null
        }

        if (
            compoundOperator != null &&
            compoundOperator in COMPOUND_OPERATORS
        ) {
            currentEnd = currentStart + 2
            currentType = KomeTokenTypes.OPERATOR
            return
        }

        currentEnd = currentStart + 1
        currentType = when (source[currentStart]) {
            '(', ')' -> KomeTokenTypes.PARENTHESES
            '{', '}' -> KomeTokenTypes.BRACES
            '[', ']' -> KomeTokenTypes.BRACKETS

            ',' -> KomeTokenTypes.COMMA
            '.' -> KomeTokenTypes.DOT
            ':' -> KomeTokenTypes.COLON

            '=', '+', '-', '*', '/',
            '!', '<', '>', '?', '|' -> {
                KomeTokenTypes.OPERATOR
            }

            else -> TokenType.BAD_CHARACTER
        }
    }

    private fun nextNonWhitespaceCharacter(
        start: Int,
    ): Char? {
        var index = start

        while (
            index < sourceEnd &&
            source[index].isWhitespace()
        ) {
            index += 1
        }

        return if (index < sourceEnd) {
            source[index]
        } else {
            null
        }
    }

    private fun startsWith(
        offset: Int,
        value: String,
    ): Boolean {
        if (offset + value.length > sourceEnd) {
            return false
        }

        for (index in value.indices) {
            if (source[offset + index] != value[index]) {
                return false
            }
        }

        return true
    }

    private fun isIdentifierStart(
        character: Char,
    ): Boolean {
        return character == '_' ||
                character.isLetter()
    }

    private fun isIdentifierContinue(
        character: Char,
    ): Boolean {
        return isIdentifierStart(character) ||
                character.isDigit()
    }

    companion object {
        private val KEYWORDS = setOf(
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
            "true",
            "false",
            "null",
        )

        private val COMPOUND_OPERATORS = setOf(
            "->",
            "=>",
            "+=",
            "==",
            "!=",
            "<=",
            ">=",
            "&&",
            "||",
        )
    }
}