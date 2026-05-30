#ifndef KOME_VIEWKIT_COMPONENTS_H
#define KOME_VIEWKIT_COMPONENTS_H

// `{"component":"text","props":{},"children":[],"content":{"type":"text","value":"..."}}`
char* __kome_viewkit_json_text(const char* value);

// `{"component":"X","props":{},"children":[...]}`
char* __kome_viewkit_json_component(const char* name, const char** children, int len);

// `dock().children(...)` のためのヘルパ
// - base から component 名だけを抽出して children を差し替える（props は今は維持しない）
char* __kome_viewkit_json_children(const char* base, const char** children, int len);

#endif /* KOME_VIEWKIT_COMPONENTS_H */
