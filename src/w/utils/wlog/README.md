# wlog.ts - TypeScript 高级日志工具

一个功能强大的 `console.log` 封装库，专为 React + Vite 项目设计，提供了日志级别控制、智能数据格式化、精准的调用位置追踪以及彩色的控制台输出。

## 功能特性

### 1. 灵活的日志级别控制

支持六个日志级别，可根据环境自动切换生产模式和开发模式：

- `OFF` - 完全禁用
- `ERROR` - 仅错误
- `WARN` - 警告和错误
- `INFO` - 信息、警告和错误
- `DEBUG` - 调试及以上
- `TRACE` - 所有日志（包括详细追踪）

### 2. 智能数据格式化（wlog_smart）

自动分析数据类型并选择最佳的显示方式：

- **字符串**：格式化为 `label: "value"`
- **数字**：格式化为 `label = 123`
- **布尔值**：显示为 `label = true/false`
- **数组对象**：使用 `console.table` 展示
- **普通对象**：根据结构自动选择表格或树形展示
- **Date 对象**：格式化为 ISO 字符串
- **Error 对象**：显示消息和堆栈信息
- **Promise**：标记为异步对象

### 3. 精准的调用位置追踪

解决传统日志封装显示 `wlog.ts:118` 的问题，直接显示实际调用点的文件名和行号。

### 4. 彩色输出

不同日志级别使用不同的颜色方案，便于快速识别：

- **INFO** - 蓝色
- **WARN** - 黄色/橙色
- **ERROR** - 红色
- **DEBUG** - 绿色
- **TRACE** - 紫色

## 安装与配置

### 1. 将文件添加到项目

将 `wlog.ts` 复制到项目的 `src/utils/` 或 `src/logger/` 目录下：

```bash
cp wlog.ts /your-project/src/utils/wlog.ts
```

### 2. 配置 Vite SourceMap（重要）

为确保调用位置追踪正常工作，需要在 `vite.config.ts` 中启用 sourceMap：

```typescript
// vite.config.ts
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  build: {
    // 启用 sourceMap 以便获得准确的错误位置
    sourcemap: true,
    // 或者只对开发环境启用
    sourcemap: false,
  },
  // 排除 wlog.ts 从构建输出中（可选）
  esbuild: {
    // 如果不想在生产包中包含 wlog
    drop: ['console', 'debugger'],
  },
});
```

### 3. 配置 Chrome DevTools 忽略列表

为了让浏览器控制台的点击链接直接跳转到你的代码而非 `wlog.ts`，需要在 Chrome DevTools 中添加忽略规则：

1. 打开 Chrome DevTools（F12）
2. 点击右上角的齿轮图标（Settings）
3. 找到 **Ignore List**（或 Blackbox）
4. 在 **Custom exclusion rules** 添加以下规则：
   - `/wlog.ts`

完成此设置后，控制台中的链接将直接指向你调用 `wlog()` 等方法的具体位置。

## 使用方法

### 基础导入

```typescript
import { wlog } from './utils/wlog';

// 或者使用默认导出
import wlog from './utils/wlog';
```

### 日志级别方法

```typescript
// 不同级别的日志输出
wlog.info('这是一条信息日志');
wlog.warn('这是一条警告日志');
wlog.error('这是一条错误日志');
wlog.debug('这是一条调试日志');
wlog.trace('这是一条追踪日志');

// 控制台输出效果：
// 14:25:30  [INFO]  [App.tsx:45] 这是一条信息日志
// 14:25:31  [WARN]  [App.tsx:46] 这是一条警告日志
// 14:25:32  [ERR ]  [App.tsx:47] 这是一条错误日志
```

### 智能日志（wlog_smart）

```typescript
// 字符串
const userName = '张三';
wlog.smart('userName', userName);
// 输出：userName: "张三"

// 数字
const age = 25;
wlog.smart('age', age);
// 输出：age = 25

// 布尔值
const isActive = true;
wlog.smart('isActive', isActive);
// 输出：isActive = true

// 对象
const user = { name: '张三', age: 25, city: '北京' };
wlog.smart('user', user);
// 输出：可折叠的表格视图

// 数组对象
const users = [
  { id: 1, name: '张三', age: 25 },
  { id: 2, name: '李四', age: 30 },
];
wlog.smart('users', users);
// 输出：可折叠的表格视图

// 空值处理
wlog.smart('maybeNull', null);
// 输出：maybeNull null（黄色背景）

wlog.smart('maybeUndefined', undefined);
// 输出：maybeUndefined undefined（黄色背景）
```

### 配置管理

```typescript
// 获取当前配置
const config = wlog.getConfig();
console.log('当前日志级别:', config.level);

// 更新配置
wlog.config({
  level: 'DEBUG',           // 设置日志级别
  showTime: true,           // 显示时间戳
  showCaller: true,         // 显示调用位置
  prefix: 'MyApp',          // 设置前缀
  prettyJSON: true,         // JSON 美化输出
  maxDepth: 5,              // 对象展开最大深度
});

// 快捷配置方法
wlog.setLevel('DEBUG');     // 设置日志级别
wlog.setSwitch(true);       // 启用/禁用日志
wlog.setPrefix('Admin');    // 设置前缀

// 重置为默认配置
wlog.reset();
```

### 环境感知的默认配置

`wlog.ts` 会自动根据环境调整配置：

```typescript
// 开发环境（import.meta.env.DEV === true）
// 默认级别：LogLevel.ALL (TRACE)

// 生产环境
// 默认级别：LogLevel.ERROR
```

你可以在项目入口文件中覆盖此行为：

```typescript
// src/main.tsx 或 src/index.tsx
import { wlog } from './utils/wlog';

// 根据环境变量设置日志级别
if (import.meta.env.PROD) {
  wlog.config({ level: LogLevel.ERROR });
}

// 或者完全禁用生产日志
if (import.meta.env.PROD) {
  wlog.setSwitch(false);
}
```

### 实用工具方法

```typescript
// 分隔线
wlog.divider();           // 使用默认字符 ─ 绘制 50 个字符的分隔线
wlog.divider('=', 30);    // 使用 = 绘制 30 个字符的分隔线

// 区块标题
wlog.section('用户模块');   // 打印带标题的装饰区块

// 表格输出
const data = [
  { name: '产品A', price: 100 },
  { name: '产品B', price: 200 },
];
wlog.table('产品价格', data);

// 性能计时
wlog.time('数据加载');
// ... 执行某些操作
wlog.timeEnd('数据加载');   // 在控制台显示耗时

// 计数器
wlog.count('点击次数');     // 每次调用递增计数器
wlog.countReset('点击次数'); // 重置计数器

// 清空控制台
wlog.clear();

// 断言
wlog.assert(user !== null, '用户不应该为空');

// JSON 格式化输出
const data = { name: '测试', value: 123 };
wlog.json('响应数据', data);
```

## 完整使用示例

```typescript
// src/components/UserProfile.tsx
import { wlog, LogLevel } from '../utils/wlog';

interface User {
  id: number;
  name: string;
  email: string;
}

export function UserProfile({ user }: { user: User }) {
  // 使用 wlog_smart 智能日志
  wlog.smart('user 参数', user);

  // 普通日志
  wlog.info('用户组件已挂载');

  // 条件日志
  if (user.id > 1000) {
    wlog.warn('用户 ID 较大，可能需要特殊处理');
  }

  // 错误日志
  try {
    // 可能抛出错误的操作
  } catch (error) {
    wlog.error('加载用户数据失败', error);
  }

  // 性能追踪
  wlog.time('用户数据处理');
  // ... 处理逻辑
  wlog.timeEnd('用户数据处理');

  return <div>{user.name}</div>;
}
```

## 与 React 开发工具集成

### 在 React Query 中使用

```typescript
import { useQuery } from '@tanstack/react-query';
import { wlog } from '../utils/wlog';

function useUserData(userId: string) {
  return useQuery({
    queryKey: ['user', userId],
    queryFn: async () => {
      wlog.time('用户查询');
      const response = await fetch(`/api/users/${userId}`);
      wlog.timeEnd('用户查询');
      return response.json();
    },
    onSuccess: (data) => {
      wlog.debug('用户数据加载成功');
      wlog.smart('用户详情', data);
    },
    onError: (error) => {
      wlog.error('用户数据加载失败', error);
    },
  });
}
```

### 在 Redux/状态管理中使用

```typescript
import { wlog } from '../utils/wlog';

// Action 日志中间件示例
const logMiddleware = (store) => (next) => (action) => {
  wlog.trace(`Action: ${action.type}`, action.payload);
  const result = next(action);
  wlog.debug(`状态更新后`, store.getState());
  return result;
};
```

## 高级定制

### 自定义颜色方案

```typescript
wlog.config({
  colors: {
    info: {
      label: '信息',
      background: '#e3f2fd',
      text: '#1565c0',
      border: '#1976d2',
    },
    warn: {
      label: '警告',
      background: '#fff3e0',
      text: '#e65100',
      border: '#ff9800',
    },
    // ... 其他级别
  },
});
```

### 根据模块设置不同前缀

```typescript
// src/utils/logger.ts
import { wlog } from './wlog';

// 创建带模块名的日志记录器
export function createLogger(prefix: string) {
  return {
    info: (msg: string) => wlog.info(`[${prefix}] ${msg}`),
    warn: (msg: string) => wlog.warn(`[${prefix}] ${msg}`),
    error: (msg: string) => wlog.error(`[${prefix}] ${msg}`),
    debug: (msg: string) => wlog.debug(`[${prefix}] ${msg}`),
    smart: (label: string, data: unknown) => wlog.smart(`${prefix}.${label}`, data),
  };
}

// 使用
import { createLogger } from '../utils/logger';
const log = createLogger('AuthModule');

log.info('用户登录成功');
log.smart('currentUser', user);
```

## 性能考虑

### 生产环境优化

在构建脚本中移除日志调用：

```typescript
// vite.config.ts
import { defineConfig } from 'vite';

export default defineConfig({
  build: {
    // 移除 console 和 debugger 语句
    minify: 'terser',
    terserOptions: {
      compress: {
        drop_console: true,
        drop_debugger: true,
      },
    },
  },
});
```

或者使用环境变量：

```typescript
// 在 wlog.ts 中
const shouldLog = !import.meta.env.PROD;

if (shouldLog) {
  // 执行日志输出
}
```

## 浏览器兼容性

`wlog.ts` 使用了以下现代 JavaScript 特性，确保在现代浏览器中正常工作：

- ES6+ 语法（类、解构赋值、箭头函数）
- `Error.stack` API（所有主流浏览器均支持）
- `console.table`（Chrome、Firefox、Safari、Edge 均支持）

对于需要支持旧版浏览器的项目，可以使用 TypeScript 编译时目标设置为 ES5，并确保配置了适当的 polyfill。

## 常见问题

### Q: 为什么控制台显示的是 wlog.ts 而不是我的文件？

这通常是因为 Chrome DevTools 没有将 `wlog.ts` 添加到忽略列表。请按照本文档「配置 Chrome DevTools 忽略列表」章节的说明进行设置。

### Q: 如何在 VS Code 中快速跳转到日志位置？

VS Code 的 JavaScript 调试器会自动处理 sourceMap。如果使用断点调试，可以在 `wlog.ts` 中设置断点，然后查看调用栈（Call Stack）面板，双击对应的栈帧即可跳转到实际调用位置。

### Q: 如何在不同环境使用不同配置？

```typescript
// src/config.ts
export const LOG_CONFIG = {
  development: {
    level: LogLevel.TRACE,
    showCaller: true,
    prettyJSON: true,
  },
  production: {
    level: LogLevel.ERROR,
    showCaller: false,
    prettyJSON: false,
  },
  test: {
    level: LogLevel.OFF,
    showCaller: false,
    prettyJSON: false,
  },
};

// src/main.tsx
import { LOG_CONFIG } from './config';
import { wlog } from './utils/wlog';

const env = import.meta.env.MODE as keyof typeof LOG_CONFIG;
wlog.config(LOG_CONFIG[env] || LOG_CONFIG.development);
```

### Q: wlog_smart 和直接使用 wlog.info 有什么区别？

`wlog.info()` 会将所有传入的参数直接传递给 `console.info`，适合打印任意消息和对象。`wlog_smart()` 则会分析数据类型并选择最佳展示方式，更适合调试变量和检查数据结构。

## API 参考

### WLog 类方法

| 方法 | 参数 | 描述 |
| ---- | ---- | ---- |
| `config()` | `Partial<WLogConfig>` | 更新配置 |
| `getConfig()` | 无 | 获取当前配置 |
| `reset()` | 无 | 重置为默认配置 |
| `setLevel()` | `LogLevel` | 设置日志级别 |
| `setSwitch()` | `boolean` | 启用/禁用日志 |
| `setPrefix()` | `string` | 设置日志前缀 |
| `info()` | `message, ...args` | 输出信息日志 |
| `warn()` | `message, ...args` | 输出警告日志 |
| `error()` | `message, ...args` | 输出错误日志 |
| `debug()` | `message, ...args` | 输出调试日志 |
| `trace()` | `message, ...args` | 输出追踪日志 |
| `smart()` | `label, data` | 智能格式化输出 |
| `divider()` | `char?, length?` | 输出分隔线 |
| `section()` | `title` | 输出区块标题 |
| `table()` | `label, data` | 表格形式输出 |
| `time()` | `label` | 启动计时器 |
| `timeEnd()` | `label` | 结束计时器 |
| `count()` | `label?` | 计数器递增 |
| `countReset()` | `label?` | 计数器重置 |
| `clear()` | 无 | 清空控制台 |
| `assert()` | `condition, message, ...args` | 断言检查 |
| `json()` | `label, data, indent?` | JSON 格式化输出 |

### LogLevel 枚举值

| 值 | 数值 | 描述 |
| ---- | ---- | ---- |
| `LogLevel.OFF` | 0 | 禁用所有日志 |
| `LogLevel.ERROR` | 1 | 仅错误 |
| `LogLevel.WARN` | 2 | 警告及以上 |
| `LogLevel.INFO` | 3 | 信息及以上 |
| `LogLevel.DEBUG` | 4 | 调试及以上 |
| `LogLevel.TRACE` | 5 | 所有日志 |

MIT License
