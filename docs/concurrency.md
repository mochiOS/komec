# Taskと非同期処理

Komeの非同期処理は`Task<T>`を中心に構成されます。`task`が処理を開始し、`wait`が完了を待ち、
`cancel`が停止を要求します。TaskはOS threadではなく、thread-localな協調scheduler上で動作します。

## Taskの作成

`task expression`はexpressionをschedulerへ登録し、直ちに`Task<T>`を返します。元のexpressionが
`T`なら、作成されるhandleの型は`Task<T>`です。

```kome
let pending: Task<Number> = task calculate()
```

task bodyは引数とcaptureした値を所有します。作成元のscopeを離れても、Taskが必要とする値は
完了または破棄まで保持されます。

## 完了待ち

`wait taskValue`は`Task<T>`から`T`を返します。対象が未完了なら現在のTaskをsuspendし、対象の
waiterとして登録します。schedulerは別のrunnable Taskを実行します。対象がterminal stateへ
移るとwaiterがwakeされ、`wait`の続きから再開します。

同じTaskを複数回waitできます。結果はTask内に保持され、各waitに対して型の通常のcopy semanticsを
適用します。runtime管理値は必要に応じてretainされます。

`Task<T>`以外への`wait`はコンパイルエラーです。cancelまたは失敗したTaskをwaitすると、値の代わりに
runtime errorを報告します。現在、Komeコードからこのerrorをcatchする構文はありません。

## Cancellation

`cancel taskValue`は`Task<T>`を受け取り、値を返しません。

| 現在の状態 | 動作 |
| --- | --- |
| Pending | 実行せずCancelledへ移行 |
| Running | cancellation requestを記録 |
| Suspended | wakeし、cleanup後にCancelledへ移行 |
| Completed | 何もしない |
| Failed | 何もしない |
| Cancelled | 何もしない |

Running Taskは協調的にcancelされます。任意の命令位置で強制停止しません。次のsuspend pointまたは
task bodyの終了時にrequestを反映します。`cancel`を複数回呼び出しても状態を逆戻りさせません。

## Taskの状態

runtimeは次の状態を区別します。

1. Pendingは作成済みで、まだ実行を開始していない状態です。
2. Runnableは実行または再開を待つ内部状態です。
3. Runningは現在schedulerが実行している状態です。
4. SuspendedはTask、timer、I/O readinessのwakeを待つ状態です。
5. CancellationRequestedは実行中に停止要求を受けた状態です。
6. Completedは結果を保持するterminal stateです。
7. Failedはerrorを保持するterminal stateです。
8. Cancelledはcancelが確定したterminal stateです。

Completed、Failed、Cancelledからactive stateへは戻りません。

## 複数Taskの待機

### `all`

`all`は1個以上の同じ結果型を持つTaskを受け取り、すべての結果を`T[]`として返します。
結果順は完了順ではなく引数順です。

```kome
let values: Number[] = all(task first(), task second())
```

いずれかがFailedまたはCancelledになると、残りのTaskへcancelを要求し、全体のerrorを報告します。
異なる`Task<T>`を混在させることはできません。

### `race`

`race`は1個以上の同じ結果型を持つTaskを受け取り、最初に正常完了した`T`を返します。勝者以外には
cancelを要求します。

Taskが1つ失敗しても、成功可能なTaskが残っていれば待機を継続します。すべてがFailedまたは
Cancelledになった場合だけ、race全体のerrorになります。

### `timeout`

`timeout(taskValue, duration)`は指定時間までTaskを待ち、成功時には`T`を返します。durationの型は
`Number`で、単位はミリ秒です。期限を超えると対象へcancelを要求し、timeout errorを報告します。

timer待機はschedulerへdeadlineを登録します。busy-loopは行いません。

## SleepとI/O readiness

標準runtimeの`io.sleep`は現在のTaskをtimerへ登録してsuspendします。sleep中も別のTaskを実行できます。

runtime-backed `Socket`はnon-blocking descriptorを所有します。`Socket.connect`、`Socket.read`、
`Socket.write`はreadinessをruntime reactorへ登録し、処理を継続できない間はTaskをsuspendします。
Linuxではepollを使用します。TaskごとにOS threadを作成しません。

`Socket.close()`は同じsocketを共有するhandleに対してもclose状態を反映し、複数回呼び出しても安全です。
readの最大byte数とportは`Number`で指定します。read結果はUTF-8の`String`です。

## Scheduler

現在のschedulerはsingle-threadedです。各Taskは独立したuser-space stackと保存済み実行contextを持ちます。
Taskがwait、timer、I/O readinessでsuspendするとscheduler contextへ戻り、runnable queueの次のTaskを
実行します。

wake処理はruntime APIへ分離されています。Task完了、timer満了、I/O readiness、cancellationは同じ
wake経路を使用します。JITとAOTは同じruntime ABIを利用します。

次の機能は現在ありません。

- 複数threadでの並列Task実行
- work stealingとTask priority
- async file I/O
- Channel、Stream、actor
- detached Taskとstructured concurrency scope
- timeout errorやTask failureをKomeコードでcatchする機構

## 所有権

Task handleと結果は参照管理されます。Task内に保存された結果は、waitされるまでTaskが所有します。
複数waitでは結果を安全に共有し、Task破棄時には未取得結果も解放します。

`all`の戻りlistは各結果の所有権を持ちます。`race`の勝者は呼び出し側へ結果を渡し、敗者がすでに
保持する結果はcancel cleanupで解放します。timeout、明示cancel、未wait Taskでも同じcleanup規則を
適用します。
