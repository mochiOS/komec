package org.mochios.kome.lang

import com.intellij.openapi.fileTypes.LanguageFileType
import javax.swing.Icon

class KomeFileType private constructor() :
    LanguageFileType(KomeLanguage) {

    override fun getName(): String {
        return "Kome"
    }

    override fun getDescription(): String {
        return "Kome source file"
    }

    override fun getDefaultExtension(): String {
        return "kome"
    }

    override fun getIcon(): Icon? {
        return null
    }

    companion object {
        @JvmField
        val INSTANCE = KomeFileType()
    }
}