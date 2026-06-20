package org.mochios.kome.lang

import com.intellij.psi.tree.IElementType

class KomeTokenType(
    debugName: String,
) : IElementType(
    debugName,
    KomeLanguage,
)

object KomeTokenTypes {
    val KEYWORD = KomeTokenType("KEYWORD")
    val IDENTIFIER = KomeTokenType("IDENTIFIER")
    val TYPE_IDENTIFIER = KomeTokenType("TYPE_IDENTIFIER")
    val CALLABLE_IDENTIFIER = KomeTokenType("CALLABLE_IDENTIFIER")

    val STRING = KomeTokenType("STRING")
    val NUMBER = KomeTokenType("NUMBER")
    val LINE_COMMENT = KomeTokenType("LINE_COMMENT")
    val ATTRIBUTE = KomeTokenType("ATTRIBUTE")

    val OPERATOR = KomeTokenType("OPERATOR")
    val PARENTHESES = KomeTokenType("PARENTHESES")
    val BRACES = KomeTokenType("BRACES")
    val BRACKETS = KomeTokenType("BRACKETS")
    val COMMA = KomeTokenType("COMMA")
    val DOT = KomeTokenType("DOT")
    val COLON = KomeTokenType("COLON")
}