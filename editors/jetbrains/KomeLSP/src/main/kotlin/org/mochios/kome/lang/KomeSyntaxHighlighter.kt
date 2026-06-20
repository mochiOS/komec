package org.mochios.kome.lang

import com.intellij.lexer.Lexer
import com.intellij.openapi.editor.DefaultLanguageHighlighterColors
import com.intellij.openapi.editor.HighlighterColors
import com.intellij.openapi.editor.colors.TextAttributesKey
import com.intellij.openapi.fileTypes.SyntaxHighlighterBase
import com.intellij.psi.TokenType
import com.intellij.psi.tree.IElementType

class KomeSyntaxHighlighter :
    SyntaxHighlighterBase() {

    override fun getHighlightingLexer(): Lexer {
        return KomeLexer()
    }

    override fun getTokenHighlights(
        tokenType: IElementType,
    ): Array<TextAttributesKey> {
        return when (tokenType) {
            KomeTokenTypes.KEYWORD -> pack(KEYWORD)
            KomeTokenTypes.IDENTIFIER -> pack(IDENTIFIER)

            KomeTokenTypes.TYPE_IDENTIFIER -> {
                pack(TYPE_IDENTIFIER)
            }

            KomeTokenTypes.CALLABLE_IDENTIFIER -> {
                pack(CALLABLE_IDENTIFIER)
            }

            KomeTokenTypes.STRING -> pack(STRING)
            KomeTokenTypes.NUMBER -> pack(NUMBER)

            KomeTokenTypes.LINE_COMMENT -> {
                pack(LINE_COMMENT)
            }

            KomeTokenTypes.ATTRIBUTE -> pack(ATTRIBUTE)
            KomeTokenTypes.OPERATOR -> pack(OPERATOR)

            KomeTokenTypes.PARENTHESES -> {
                pack(PARENTHESES)
            }

            KomeTokenTypes.BRACES -> pack(BRACES)
            KomeTokenTypes.BRACKETS -> pack(BRACKETS)
            KomeTokenTypes.COMMA -> pack(COMMA)
            KomeTokenTypes.DOT -> pack(DOT)
            KomeTokenTypes.COLON -> pack(OPERATOR)

            TokenType.BAD_CHARACTER -> {
                pack(BAD_CHARACTER)
            }

            else -> emptyArray()
        }
    }

    companion object {
        val KEYWORD = TextAttributesKey.createTextAttributesKey(
            "KOME_KEYWORD",
            DefaultLanguageHighlighterColors.KEYWORD,
        )

        val IDENTIFIER = TextAttributesKey.createTextAttributesKey(
            "KOME_IDENTIFIER",
            DefaultLanguageHighlighterColors.IDENTIFIER,
        )

        val TYPE_IDENTIFIER =
            TextAttributesKey.createTextAttributesKey(
                "KOME_TYPE_IDENTIFIER",
                DefaultLanguageHighlighterColors.CLASS_NAME,
            )

        val CALLABLE_IDENTIFIER =
            TextAttributesKey.createTextAttributesKey(
                "KOME_CALLABLE_IDENTIFIER",
                DefaultLanguageHighlighterColors.FUNCTION_CALL,
            )

        val STRING = TextAttributesKey.createTextAttributesKey(
            "KOME_STRING",
            DefaultLanguageHighlighterColors.STRING,
        )

        val NUMBER = TextAttributesKey.createTextAttributesKey(
            "KOME_NUMBER",
            DefaultLanguageHighlighterColors.NUMBER,
        )

        val LINE_COMMENT =
            TextAttributesKey.createTextAttributesKey(
                "KOME_LINE_COMMENT",
                DefaultLanguageHighlighterColors.LINE_COMMENT,
            )

        val ATTRIBUTE = TextAttributesKey.createTextAttributesKey(
            "KOME_ATTRIBUTE",
            DefaultLanguageHighlighterColors.METADATA,
        )

        val OPERATOR = TextAttributesKey.createTextAttributesKey(
            "KOME_OPERATOR",
            DefaultLanguageHighlighterColors.OPERATION_SIGN,
        )

        val PARENTHESES =
            TextAttributesKey.createTextAttributesKey(
                "KOME_PARENTHESES",
                DefaultLanguageHighlighterColors.PARENTHESES,
            )

        val BRACES = TextAttributesKey.createTextAttributesKey(
            "KOME_BRACES",
            DefaultLanguageHighlighterColors.BRACES,
        )

        val BRACKETS = TextAttributesKey.createTextAttributesKey(
            "KOME_BRACKETS",
            DefaultLanguageHighlighterColors.BRACKETS,
        )

        val COMMA = TextAttributesKey.createTextAttributesKey(
            "KOME_COMMA",
            DefaultLanguageHighlighterColors.COMMA,
        )

        val DOT = TextAttributesKey.createTextAttributesKey(
            "KOME_DOT",
            DefaultLanguageHighlighterColors.DOT,
        )

        val BAD_CHARACTER =
            TextAttributesKey.createTextAttributesKey(
                "KOME_BAD_CHARACTER",
                HighlighterColors.BAD_CHARACTER,
            )
    }
}