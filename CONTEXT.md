# LiveTranslate-rs 领域词汇表

实时音频翻译应用的单一上下文词汇表:只收本项目特有领域概念的定名与消歧,不收通用编程概念。实现细节(文件、行号、机制)一律不进本文;机制与真源走 AGENTS.md 路由表。

## Language

**settings 键**:
设置界面与持久层(settings.json)使用的引擎名,值域恒为 funasr / whisper / qwen3 的字符串。对外的、用户可见的那套引擎词表。
_Avoid_: 引擎名(泛称)、worker 名

**EngineKey**:
settings 键的类型化身份透镜,三变体 FunAsr / Whisper / Qwen3。域内一切「这是哪个引擎」的判定都应经过它,而非裸字符串比较。
_Avoid_: 引擎枚举(泛称)

**worker 身份**:
worker 子进程分派加载哪个引擎实现的身份。与 settings 键是两套词表:funasr 在 settings 侧是一个键,在 worker 侧拆分为 SenseVoice 与 Nano 两个身份;施工后由 WorkerEngine 枚举承载(D-116)。
_Avoid_: 引擎 key 裸用、family 键

**引擎家族**:
worker 身份的族级分组:FunASR 家族 = SenseVoice + Nano;Whisper、Qwen3 各自成族。padding 分派等族级行为按家族走。
_Avoid_: 引擎类型、引擎组

**超时档案**:
按引擎身份给识别请求的动态超时预算:「base + 每秒段长 × 段长」。档案消费 settings 级身份;FunASR 家族共用一份快档,与 qwen3 的慢档区分。
_Avoid_: 超时配置、超时表

**草稿写入面**:
设置类命令对运行时设置草稿的唯一写入落点。UI 只发命令、不预写草稿;命令到达宿主后经此落点生效,防抖后统一落盘。
_Avoid_: 副作用表、直接写设置
