# Kome language documentation

> English documentation is currently being prepared.

### Kome言語とは？

Kome言語は、主にLinuxおよび[mochiOS](https://mochios.github.io/)を対象に開発された、シンプルで効率的なプログラミング言語です。
Komeはモダンな構文と強力な機能を提供し、アプリケーション開発を迅速に、かつとても簡単に行うことができます。

### Kome言語の特徴
- **シンプルな構文**: Komeは、読みやすく書きやすい構文を採用しており、初心者から経験豊富な開発者まで幅広く利用できます。
- **FFIサポート**: Komeは、C言語などの他の言語との相互運用性を提供するFFI（Foreign Function Interface）をサポートしています。
- **とりあえずめっちゃ簡単**: Komeは、複雑な機能をシンプルに提供することを目指しており、開発者がすぐに使い始めることができるように設計されています。

### 例

とりあえず、Komeの基本的な構文を示す簡単な例を以下に示します。
```rust
fn main() {
    println("Hello, World!");
}
```

この例では、`main`関数が定義されており、`println`関数を使用してコンソールに「Hello, World!」と出力しています。
とてつもなくRustっぽい構文ですが、KomeはRustとは異なる独自の言語であることに注意してください。

また、Kome言語には「State」という概念があり、これを使用して状態管理を行うことができます。

```rust
use std.io
use std.io.keyboard
use std.bundle

bundle App {
    public state let mut Index = 0

    recipe button: Index = {
        printf("Button pressed: %d\n", Index)
    }
}

fn main() {
    keyboard.scan(any) {
        App.Index += 1          // キーが押されるたびにIndexをインクリメント
    }

    App.run()
}
```

この例では、`App`というバンドルが定義されており、その中に`Index`という状態が宣言されています。
キーボードの任意のキーが押されるたびに、`Index`がインクリメントされ、それに紐付けられているレシピ（関数のようなもの）が実行されます。

つまり、わざわざ常に変数が更新されたかを開発者は気にする必要がなく、状態が更新されるたびに自動的に関連する処理が実行されるため、非常にシンプルで効率的なコードを書くことができます。

そして、直感的な構文（英文法に近い構文）もKomeの特徴の一つです。
```rust
use viewKit

bundle App {
  public state let mut clicked = 0;

  // stateで定義された変数が指定されていたら変更されたら再評価される
  // recipe レシピ名: 参照する常態みたいに
  recipe card: clicked = card().children(
     is clicked 0 => text("Hello, world!")
     is clicked 1 => text("Clicked!")
  )
}

fn main() {
  let mut window = window.create() with children(
    card: App.card,
  )

  window.card.onClick {
    !default clicked = 0;  // この条件でないときはこれになる
    clicked = 1;           // クリックされたらclickedを1にする
  }
}
```

この例では、`App`バンドル内に`clicked`という状態が定義されており、`is clicked 0`や`is clicked 1`のような条件式を使用して、状態に応じたUIの表示を切り替えています。
さらに、`window.card.onClick`のようなイベントハンドラーを使用して、クリックイベントに応じて状態を更新することができます。
`!default`は、条件が満たされない場合のデフォルトの値を指定するために使用されます。
`clicked`が0のときは「Hello, world!」が表示され、1のときは「Clicked!」が表示されるようになっています。