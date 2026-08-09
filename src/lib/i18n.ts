import i18next from "i18next";
import { initReactI18next } from "react-i18next";

export const RESOURCE_LOCALES = ["en-US", "zh-CN"] as const;
export type ResourceLocale = (typeof RESOURCE_LOCALES)[number];
export type InterfaceLocalePreference = "system" | ResourceLocale;

export const INTERFACE_LOCALE_STORAGE_KEY = "adaq.interfaceLocale";

const english = {
	app: {
		initializing: "AdaQ is initializing",
		workspaceInitializing: "Initializing workspace…",
	},
	nav: {
		dashboard: "Dashboard",
		components: "Components",
		models: "Models",
		backtest: "Backtest",
		validation: "Validation",
		settings: "Settings",
		github: "GitHub",
		wechat: "WeChat",
		account: "Account",
		logOut: "Log out",
		signedIn: "Signed in",
	},
	titlebar: {
		back: "Back",
		forward: "Forward",
		toggleSidebar: "Toggle Sidebar",
		theme: "Theme: {{theme}}",
		themeLabel: "Theme",
		checkForUpdates: "Check for Updates",
	},
	theme: {
		light: "Light",
		dark: "Dark",
		system: "System",
	},
	market: {
		watchlist: "Watchlist",
		updatedAt: "Updated {{time}}",
		instrumentVenue: "{{baseAsset}} / {{quoteAsset}} · OKX Spot",
		bid: "Bid",
		ask: "Ask",
		high24h: "24h High",
		low24h: "24h Low",
		loadingTicker: "Loading {{instrument}} ticker…",
		live: "Live",
		liveWebSocket: "Live WebSocket",
		connecting: "Connecting",
		reconnecting: "Reconnecting",
		volume24h:
			"24h volume {{baseVolume}} {{baseAsset}} · {{quoteVolume}} {{quoteAsset}}",
		watchlistSummary: "{{count}} / {{limit}} · OKX Spot",
		miniChartInterval: "Mini-chart interval",
		add: "Add",
		instrument: "Instrument",
		chart: "Chart",
		price: "Price",
		change: "% Chg",
		actions: "Actions",
		loadingWatchlist: "Loading Watchlist…",
		emptyWatchlist: "Your Watchlist is empty.",
		pageOf: "Page {{current}} of {{total}}",
		searchInstruments: "Search OKX Spot",
		closeInstrumentSearch: "Close instrument search",
		loadingInstruments: "Loading Instruments…",
		noAvailableMatches: "No available matches.",
		unavailable: "Unavailable",
		removeFromWatchlist: "Remove {{instrument}} from Watchlist",
		barInterval: "Bar interval",
		reset: "Reset",
		loadingBars: "Loading OKX bars…",
		retrying: "Retrying…",
		retry: "Retry",
		noClosedBars: "No closed bars returned by OKX.",
		loadingHistory: "Loading history…",
		updating: "Updating…",
		chartAriaLabel: "{{instrument}} {{interval}} candlestick chart from OKX",
		publicMarketData: "Public market data from OKX",
		utc: "UTC",
		open: "Open",
		high: "High",
		low: "Low",
		close: "Close",
		baseVolume: "Base volume",
		quoteVolume: "Quote volume",
	},
	sidebar: {
		title: "Sidebar",
		description: "Displays the mobile sidebar.",
	},
	settings: {
		navigation: {
			backToApp: "Back to App",
			label: "Settings",
			general: "General",
			profile: "Profile",
			appearance: "Appearance",
			keyboardShortcuts: "Keyboard Shortcuts",
			account: "Account",
			connections: "Connections",
			dataStorage: "Data & Storage",
		},
		general: {
			title: "General",
			description: "Application updates and version information.",
			language: {
				title: "Interface language",
				description:
					"Choose the language used for ADAQ's interface on this device.",
				label: "Interface language",
				system: "System",
				englishUS: "English (US)",
				simplifiedChinese: "简体中文",
			},
			updates: {
				title: "Software Updates",
				description: "Keep ADAQ current with signed application releases.",
				autoDownload: "Automatically download updates",
				autoDownloadDescription:
					"Check at startup and download an available update.",
				version: "Version {{version}}",
				loading: "Loading…",
				development: "Development",
				unavailable: "Unavailable",
				versionDescription: "Installed application version.",
				check: "Check for updates",
			},
			disclaimer: {
				title: "Disclaimer",
				text:
					"This software is for educational and research purposes only. It is not financial advice, and nothing in it constitutes a recommendation to buy, sell, or hold any security or digital asset. Historical performance and simulated backtest results do not guarantee future results. You use this software at your own risk. The authors and contributors shall not be liable for any direct, indirect, incidental, consequential, or special damages, including but not limited to financial losses, arising from the use of or inability to use this software.",
			},
		},
		profile: {
			title: "Profile",
			description: "Your presentation identity in ADAQ.",
			avatarDescription: "Your connected account avatar is used when available.",
			avatarAlt: "Profile",
			displayName: "Display name",
			save: "Save profile",
			saved: "Profile saved.",
		},
		appearance: {
			title: "Appearance",
			description: "Choose how ADAQ looks on this device.",
			theme: "Theme",
		},
		keyboard: {
			title: "Keyboard Shortcuts",
			description: "Available application shortcuts.",
			toggleSidebar: "Toggle Sidebar",
			toggleSidebarDescription: "Show or hide the workspace sidebar.",
			reloadPage: "Reload Page",
			reloadPageDescription: "Reload the current application window.",
			zoomIn: "Zoom In",
			zoomInDescription: "Increase the interface zoom level.",
			zoomOut: "Zoom Out",
			zoomOutDescription: "Decrease the interface zoom level.",
			resetZoom: "Reset Zoom",
			resetZoomDescription: "Restore the default interface zoom level.",
		},
		account: {
			title: "Account",
			description: "Authentication details and session actions.",
			email: "Email",
			emailDescription: "Your account email cannot be changed here.",
			loading: "Loading…",
			changePassword: "Change password",
			changePasswordDescription:
				"Confirm your current password before choosing a new one.",
			currentPassword: "Current password",
			newPassword: "New password",
			confirmPassword: "Confirm new password",
			passwordsMatch: "Passwords match",
			changePasswordAction: "Change password",
			logOut: "Log out",
			logOutDescription: "End the current ADAQ session.",
			currentPasswordIncorrect: "Current password is incorrect.",
			passwordChanged: "Password changed.",
		},
		dataStorage: {
			title: "Data & Storage",
			description: "Inspect and reset local research data for this User.",
			localStorage: "Local storage",
			loading: "Loading local data…",
			resetLocalData: "Reset local data",
			resetDescription:
				"These actions cannot be undone. Account and interface preferences are preserved.",
			database: "Database",
			componentPackages: "Component Packages",
			marketData: "Market Data",
			resetWatchlist: "Reset Watchlist",
			resetWatchlistDescription: "Restore BTC-USDT, ETH-USDT, and SOL-USDT.",
			resetComponents: "Reset Component Packages",
			resetComponentsDescription:
				"Remove local Component Package access and unreferenced files.",
			resetMarketData: "Reset Market Data",
			resetMarketDataDescription:
				"Remove local Market Data Snapshot access and unreferenced Parquet files.",
			resetAll: "Reset All Local Research Data",
			resetAllDescription:
				"Remove this User's Watchlist, Components, Model Artifacts, Market Data, Generation Attempts, Signal Datasets, Runs, Protocols, and Reports.",
			resetButton: "Reset",
			completed: "{{title}} completed.",
			confirmTitle: "Confirm {{title}}",
			confirmDescription:
				"This action affects only the current User and cannot be undone.",
			blocked:
				"This reset is blocked by immutable research records. Use Reset All to remove the complete dependency chain.",
			typeReset: "Type RESET to continue",
			cancel: "Cancel",
			dataToReset: "Data to reset",
			watchlistItems: "Watchlist items: {{count}}",
			componentPackagesCount: "Component Packages: {{count}}",
			marketDataSnapshotsCount: "Market Data Snapshots: {{count}}",
			backtestRuns: "Backtest Runs: {{count}}",
			validationProtocols: "Validation Protocols: {{count}}",
			validationReports: "Validation Reports: {{count}}",
			generationAttempts: "Generation Attempts: {{count}}",
			modelArtifacts: "Model Artifact registrations: {{count}}",
			signalDatasets: "Forecast Signal Datasets: {{count}}",
			preserved:
				"Preserved: login, Account, Profile, theme, and update preference.",
		},
		connections: {
			title: "Connections",
			description:
				"Paper/Demo provider credentials are stored in the operating-system secret store on this device only.",
			credentialEntryHint:
				"Enter Alpaca Paper Key ID and Secret Key only in Settings > Connections. Never paste them into chat or a .env file; the Host stores them in the operating-system secret store.",
			requiresDesktop: "Connections are managed in the ADAQ desktop app.",
			loading: "Loading connections…",
			savedSecretHint:
				"The saved credential is never redisplayed. Re-enter every value to rotate.",
			keyPlaceholder: "Required to rotate",
			secretPlaceholder: "Required to rotate",
			environment: "Environment",
			maskedSuffix: "Key ending",
			accountId: "Account ID",
			currency: "Currency",
			lastTest: "Last test",
			neverTested: "Never tested",
			capabilities: "Capabilities",
			status: {
				usable: "Usable",
				unusable: "Unusable",
			},
			alpacaPaper: {
				title: "Alpaca Paper",
				description:
					"U.S. equities paper trading on the fixed Alpaca Paper environment; Basic market data is IEX-only.",
				keyId: "Paper API Key ID",
				secretKey: "Paper Secret Key",
			},
			okxDemo: {
				title: "OKX Demo Trading",
				description:
					"Crypto spot demo trading on the fixed OKX Demo environment with simulated orders.",
				apiKey: "Demo API Key",
				secretKey: "Secret Key",
				passphrase: "Passphrase",
			},
			aShare: {
				title: "A-share Paper",
				description:
					"Uses the local ADAQ Ordinary Securities Account simulator; no broker credential is needed.",
			},
			save: "Save & test",
			rotate: "Save & rotate",
			test: "Test again",
			delete: "Delete",
			deleteConfirm:
				"Delete this connection? The operating-system credential will be removed and the Profile invalidated.",
			saved: "Connection saved and tested.",
			tested: "Connection test completed.",
			deleted: "Connection deleted.",
			errors: {
				unknown: "The connection could not be completed.",
				invalid_input: "Credential values are missing or too long.",
				auth_failed: "The provider rejected these credentials.",
				inactive_account: "The provider account is not active.",
				environment_mismatch:
					"Environment mismatch: the key is not valid for the fixed Paper/Demo environment.",
				currency_mismatch:
					"The account currency differs from the expected Paper currency.",
				account_mismatch:
					"The provider account identity or currency changed since the confirmed binding.",
				clock_skew: "The device clock is out of sync with the provider.",
				missing_permission:
					"The key is missing a required permission.",
				withdrawal_capability:
					"This key has withdrawal capability; V1 requires Read/Trade only. Create a least-privilege key.",
				missing_reference:
					"The stored credential is missing; re-save the connection.",
				secret_store_unavailable:
					"The operating-system secret store is unavailable.",
				request_failed: "The provider request failed.",
				blocked_active_runtime:
					"Deletion is blocked while an active runtime depends on this connection.",
				invalid_profile: "The connection profile was not found.",
				internal: "An internal error occurred.",
			},
		},
	},
	loading: {
		page: "Loading page…",
		modelPackages: "Loading Model Packages…",
		marketDataSnapshots: "Loading Market Data Snapshots…",
		generationAttempts: "Loading Generation Attempts…",
		signalDatasets: "Loading Signal Datasets…",
		forecastEvaluationReports: "Loading Forecast Evaluation Reports…",
		completedRuns: "Loading Completed Runs…",
		protocols: "Loading Protocols…",
		reports: "Loading Reports…",
		readableSnapshots: "Loading readable Snapshots…",
		runHistory: "Loading Run History…",
		snapshots: "Loading Snapshots…",
		componentPackages: "Loading Component Packages…",
		strategyComponents: "Loading Strategy Components…",
		validatingPackage: "Validating package…",
		removing: "Removing…",
		signalRows: "Loading Signal rows…",
		evaluating: "Evaluating…",
		loadingRun: "Loading Run…",
		freezing: "Freezing…",
		running: "Running…",
		exportingJson: "Exporting JSON…",
		exportingMarkdown: "Exporting Markdown…",
		preparingSnapshot: "Preparing Snapshot…",
		pagedExecutionEvidence: "Loading paged execution evidence…",
		downloadingClosedBars: "Downloading and freezing Closed Bars…",
		exactValidationEvidence: "Loading exact validation evidence…",
	},
	updates: {
		latestVersion: "You are using the latest version v{{version}}.",
		latestVersionFallback: "You are using the latest version.",
		checkFailed: "Unable to check for updates. Please try again later.",
		updateTo: "Update to v{{version}}",
		update: "Update",
		downloading: "Downloading",
		percentDownloading: "{{percent}}% Downloading...",
		restart: "Restart to update",
		retry: "Retry update",
	},
	auth: {
		supabaseNotConfigured: "Supabase is not configured",
		supabaseNotConfiguredDescription:
			"Set VITE_SUPABASE_URL and VITE_SUPABASE_PUBLISHABLE_KEY, then restart the app.",
		initializingAria: "AdaQ is initializing",
		initializingWorkspace: "Initializing workspace…",
		signIn: "Sign in",
		signInWithPassword: "Sign in with password",
		enterVerificationCode: "Enter verification code",
		createStrongPassword: "Create a strong password",
		secureAccount: "Secure your account before continuing.",
		checkEmail: "Check your email for the 8-digit code.",
		passwordSignInNoSession:
			"Password sign-in succeeded, but no session was returned.",
		codeAcceptedNoSession: "The code was accepted, but no session was returned.",
		continue: "Continue",
		verifyCode: "Verify code",
		differentEmail: "Use a different email",
		emailCodeInstead: "Sign in with email code instead",
		emailDescription:
			"Please input your email address. For new accounts, an email with a OTP code will be sent to your email address. Existing accounts continue with password.",
		email: "Email",
		password: "Password",
		code: "Code",
		confirmPassword: "Confirm password",
		createPassword: "Create password",
		passwordsMatch: "Passwords match",
		passwordRequirements: {
			length: "At least 8 characters",
			lowercase: "Lowercase letter",
			uppercase: "Uppercase letter",
			digit: "Digit",
			symbol: "Symbol",
		},
	},
} as const;

const simplifiedChinese = {
	app: {
		initializing: "AdaQ 正在初始化",
		workspaceInitializing: "正在初始化工作区…",
	},
	nav: {
		dashboard: "仪表盘",
		components: "组件",
		models: "模型",
		backtest: "回测",
		validation: "验证",
		settings: "设置",
		github: "GitHub",
		wechat: "微信",
		account: "账户",
		logOut: "退出登录",
		signedIn: "已登录",
	},
	titlebar: {
		back: "后退",
		forward: "前进",
		toggleSidebar: "切换侧边栏",
		theme: "主题：{{theme}}",
		themeLabel: "主题",
		checkForUpdates: "检查更新",
	},
	theme: {
		light: "浅色",
		dark: "深色",
		system: "跟随系统",
	},
	market: {
		watchlist: "自选列表",
		updatedAt: "更新于 {{time}}",
		instrumentVenue: "{{baseAsset}} / {{quoteAsset}} · OKX Spot",
		bid: "买价",
		ask: "卖价",
		high24h: "24 小时最高",
		low24h: "24 小时最低",
		loadingTicker: "正在加载 {{instrument}} 行情…",
		live: "实时",
		liveWebSocket: "实时 WebSocket",
		connecting: "正在连接",
		reconnecting: "正在重新连接",
		volume24h:
			"24 小时成交量 {{baseVolume}} {{baseAsset}} · {{quoteVolume}} {{quoteAsset}}",
		watchlistSummary: "{{count}} / {{limit}} · OKX Spot",
		miniChartInterval: "迷你图周期",
		add: "添加",
		instrument: "交易品种",
		chart: "图表",
		price: "价格",
		change: "涨跌幅",
		actions: "操作",
		loadingWatchlist: "正在加载自选列表…",
		emptyWatchlist: "您的自选列表为空。",
		pageOf: "第 {{current}} / {{total}} 页",
		searchInstruments: "搜索 OKX Spot",
		closeInstrumentSearch: "关闭交易品种搜索",
		loadingInstruments: "正在加载交易品种…",
		noAvailableMatches: "没有可用的匹配项。",
		unavailable: "不可用",
		removeFromWatchlist: "从自选列表移除 {{instrument}}",
		barInterval: "K 线周期",
		reset: "重置",
		loadingBars: "正在加载 OKX K 线…",
		retrying: "正在重试…",
		retry: "重试",
		noClosedBars: "OKX 未返回已关闭 K 线。",
		loadingHistory: "正在加载历史数据…",
		updating: "正在更新…",
		chartAriaLabel: "来自 OKX 的 {{instrument}} {{interval}} K 线图",
		publicMarketData: "来自 OKX 的公开市场数据",
		utc: "UTC",
		open: "开盘",
		high: "最高",
		low: "最低",
		close: "收盘",
		baseVolume: "基础资产成交量",
		quoteVolume: "计价资产成交量",
	},
	sidebar: {
		title: "侧边栏",
		description: "显示移动端侧边栏。",
	},
	settings: {
		navigation: {
			backToApp: "返回应用",
			label: "设置",
			general: "通用",
			profile: "个人资料",
			appearance: "外观",
			keyboardShortcuts: "键盘快捷键",
			account: "账户",
			connections: "连接",
			dataStorage: "数据与存储",
		},
		general: {
			title: "通用",
			description: "应用更新和版本信息。",
			language: {
				title: "界面语言",
				description: "选择此设备上 ADAQ 使用的界面语言。",
				label: "界面语言",
				system: "系统",
				englishUS: "English (US)",
				simplifiedChinese: "简体中文",
			},
			updates: {
				title: "软件更新",
				description: "通过签名的应用版本保持 ADAQ 为最新状态。",
				autoDownload: "自动下载更新",
				autoDownloadDescription: "启动时检查并下载可用更新。",
				version: "版本 {{version}}",
				loading: "正在加载…",
				development: "开发版本",
				unavailable: "不可用",
				versionDescription: "已安装的应用版本。",
				check: "检查更新",
			},
			disclaimer: {
				title: "免责声明",
				text:
					"本软件仅用于教育和研究目的，不构成财务建议，软件中的任何内容都不构成买入、卖出或持有任何证券或数字资产的建议。历史表现和模拟回测结果不保证未来结果。使用本软件的风险由您自行承担。对于因使用或无法使用本软件而产生的任何直接、间接、附带、后果性或特殊损害（包括但不限于财务损失），作者和贡献者不承担责任。",
			},
		},
		profile: {
			title: "个人资料",
			description: "您在 ADAQ 中的展示身份。",
			avatarDescription: "有可用时使用您关联账户的头像。",
			avatarAlt: "个人资料",
			displayName: "显示名称",
			save: "保存个人资料",
			saved: "个人资料已保存。",
		},
		appearance: {
			title: "外观",
			description: "选择此设备上的 ADAQ 外观。",
			theme: "主题",
		},
		keyboard: {
			title: "键盘快捷键",
			description: "可用的应用快捷键。",
			toggleSidebar: "切换侧边栏",
			toggleSidebarDescription: "显示或隐藏工作区侧边栏。",
			reloadPage: "重新加载页面",
			reloadPageDescription: "重新加载当前应用窗口。",
			zoomIn: "放大",
			zoomInDescription: "增大界面缩放级别。",
			zoomOut: "缩小",
			zoomOutDescription: "减小界面缩放级别。",
			resetZoom: "重置缩放",
			resetZoomDescription: "恢复默认界面缩放级别。",
		},
		account: {
			title: "账户",
			description: "认证详情和会话操作。",
			email: "邮箱",
			emailDescription: "无法在此处修改您的账户邮箱。",
			loading: "正在加载…",
			changePassword: "修改密码",
			changePasswordDescription: "请确认当前密码后再选择新密码。",
			currentPassword: "当前密码",
			newPassword: "新密码",
			confirmPassword: "确认新密码",
			passwordsMatch: "密码匹配",
			changePasswordAction: "修改密码",
			logOut: "退出登录",
			logOutDescription: "结束当前 ADAQ 会话。",
			currentPasswordIncorrect: "当前密码不正确。",
			passwordChanged: "密码已修改。",
		},
		dataStorage: {
			title: "数据与存储",
			description: "查看和重置此 User 的本地研究数据。",
			localStorage: "本地存储",
			loading: "正在加载本地数据…",
			resetLocalData: "重置本地数据",
			resetDescription: "这些操作无法撤销。账户和界面偏好会保留。",
			database: "数据库",
			componentPackages: "组件包",
			marketData: "市场数据",
			resetWatchlist: "重置自选列表",
			resetWatchlistDescription: "恢复 BTC-USDT、ETH-USDT 和 SOL-USDT。",
			resetComponents: "重置组件包",
			resetComponentsDescription: "移除本地组件包访问权限和未引用文件。",
			resetMarketData: "重置市场数据",
			resetMarketDataDescription:
				"移除本地市场数据快照访问权限和未引用的 Parquet 文件。",
			resetAll: "重置全部本地研究数据",
			resetAllDescription:
				"移除此 User 的自选列表、组件、模型产物、市场数据、生成尝试、信号数据集、运行、协议和报告。",
			resetButton: "重置",
			completed: "{{title}}已完成。",
			confirmTitle: "确认 {{title}}",
			confirmDescription: "此操作只影响当前 User，且无法撤销。",
			blocked: "此重置被不可变研究记录阻止。请使用“重置全部”移除完整依赖链。",
			typeReset: "输入 RESET 继续",
			cancel: "取消",
			dataToReset: "将要重置的数据",
			watchlistItems: "自选列表项目：{{count}}",
			componentPackagesCount: "组件包：{{count}}",
			marketDataSnapshotsCount: "市场数据快照：{{count}}",
			backtestRuns: "回测运行：{{count}}",
			validationProtocols: "验证协议：{{count}}",
			validationReports: "验证报告：{{count}}",
			generationAttempts: "生成尝试：{{count}}",
			modelArtifacts: "模型产物注册：{{count}}",
			signalDatasets: "预测信号数据集：{{count}}",
			preserved: "保留：登录、账户、个人资料、主题和更新偏好。",
		},
		connections: {
			title: "连接",
			description: "Paper/Demo 提供商凭据仅存储在此设备的操作系统密钥库中。",
			credentialEntryHint:
				"Alpaca Paper Key ID 和 Secret Key 只能填写在“设置 > 连接”中。不要粘贴到聊天或 .env 文件；Host 会将其存储在操作系统密钥库中。",
			requiresDesktop: "连接在 ADAQ 桌面应用中管理。",
			loading: "正在加载连接…",
			savedSecretHint: "已保存的凭据不会重新显示。轮换时需要重新输入全部值。",
			keyPlaceholder: "轮换时必填",
			secretPlaceholder: "轮换时必填",
			environment: "环境",
			maskedSuffix: "密钥结尾",
			accountId: "账户 ID",
			currency: "货币",
			lastTest: "上次测试",
			neverTested: "从未测试",
			capabilities: "能力",
			status: {
				usable: "可用",
				unusable: "不可用",
			},
			alpacaPaper: {
				title: "Alpaca Paper",
				description: "在固定的 Alpaca Paper 环境中进行美股纸面交易；Basic 行情仅为 IEX。",
				keyId: "Paper API Key ID",
				secretKey: "Paper Secret Key",
			},
			okxDemo: {
				title: "OKX Demo 交易",
				description: "在固定的 OKX Demo 环境中以模拟订单进行加密现货演示交易。",
				apiKey: "Demo API Key",
				secretKey: "Secret Key",
				passphrase: "Passphrase",
			},
			aShare: {
				title: "A 股纸面账户",
				description: "使用 ADAQ 本地的普通证券账户模拟器；无需券商凭据。",
			},
			save: "保存并测试",
			rotate: "保存并轮换",
			test: "重新测试",
			delete: "删除",
			deleteConfirm: "删除此连接？操作系统密钥库中的凭据将被移除，Profile 将失效。",
			saved: "连接已保存并通过测试。",
			tested: "连接测试已完成。",
			deleted: "连接已删除。",
			errors: {
				unknown: "无法完成连接。",
				invalid_input: "凭据缺失或过长。",
				auth_failed: "提供商拒绝了这些凭据。",
				inactive_account: "提供商账户未激活。",
				environment_mismatch: "环境不匹配：该密钥不适用于固定的 Paper/Demo 环境。",
				currency_mismatch: "账户货币与预期的 Paper 货币不一致。",
				account_mismatch: "提供商账户身份或货币与已确认的绑定不一致。",
				clock_skew: "设备时钟与提供商时间不同步。",
				missing_permission: "密钥缺少所需权限。",
				withdrawal_capability: "该密钥具备提现能力；V1 仅允许 Read/Trade。请创建最小权限密钥。",
				missing_reference: "已存储的凭据丢失；请重新保存连接。",
				secret_store_unavailable: "操作系统密钥库不可用。",
				request_failed: "提供商请求失败。",
				blocked_active_runtime: "有活动运行时依赖此连接，删除被阻止。",
				invalid_profile: "未找到连接 Profile。",
				internal: "发生内部错误。",
			},
		},
	},
	loading: {
		page: "正在加载页面…",
		modelPackages: "正在加载模型包…",
		marketDataSnapshots: "正在加载市场数据快照…",
		generationAttempts: "正在加载生成尝试…",
		signalDatasets: "正在加载信号数据集…",
		forecastEvaluationReports: "正在加载预测评估报告…",
		completedRuns: "正在加载已完成的运行…",
		protocols: "正在加载协议…",
		reports: "正在加载报告…",
		readableSnapshots: "正在加载可读快照…",
		runHistory: "正在加载运行历史…",
		snapshots: "正在加载快照…",
		componentPackages: "正在加载组件包…",
		strategyComponents: "正在加载策略组件…",
		validatingPackage: "正在验证包…",
		removing: "正在移除…",
		signalRows: "正在加载信号行…",
		evaluating: "正在评估…",
		loadingRun: "正在加载运行…",
		freezing: "正在冻结…",
		running: "正在运行…",
		exportingJson: "正在导出 JSON…",
		exportingMarkdown: "正在导出 Markdown…",
		preparingSnapshot: "正在准备快照…",
		pagedExecutionEvidence: "正在加载分页执行证据…",
		downloadingClosedBars: "正在下载并冻结已关闭 K 线…",
		exactValidationEvidence: "正在加载精确验证证据…",
	},
	updates: {
		latestVersion: "您正在使用最新版本 v{{version}}。",
		latestVersionFallback: "您正在使用最新版本。",
		checkFailed: "无法检查更新，请稍后重试。",
		updateTo: "更新到 v{{version}}",
		update: "更新",
		downloading: "正在下载",
		percentDownloading: "{{percent}}% 正在下载…",
		restart: "重启以更新",
		retry: "重试更新",
	},
	auth: {
		supabaseNotConfigured: "Supabase 尚未配置",
		supabaseNotConfiguredDescription:
			"设置 VITE_SUPABASE_URL 和 VITE_SUPABASE_PUBLISHABLE_KEY，然后重启应用。",
		initializingAria: "AdaQ 正在初始化",
		initializingWorkspace: "正在初始化工作区…",
		signIn: "登录",
		signInWithPassword: "使用密码登录",
		enterVerificationCode: "输入验证码",
		createStrongPassword: "创建强密码",
		secureAccount: "请先保护您的账户。",
		checkEmail: "请查看您的邮箱，获取 8 位验证码。",
		passwordSignInNoSession: "密码登录成功，但未返回会话。",
		codeAcceptedNoSession: "验证码已接受，但未返回会话。",
		continue: "继续",
		verifyCode: "验证验证码",
		differentEmail: "使用其他邮箱",
		emailCodeInstead: "改用邮箱验证码登录",
		emailDescription:
			"请输入您的邮箱地址。新账户会收到包含验证码的邮件，已有账户将继续使用密码登录。",
		email: "邮箱",
		password: "密码",
		code: "验证码",
		confirmPassword: "确认密码",
		createPassword: "创建密码",
		passwordsMatch: "密码匹配",
		passwordRequirements: {
			length: "至少 8 个字符",
			lowercase: "小写字母",
			uppercase: "大写字母",
			digit: "数字",
			symbol: "符号",
		},
	},
} as const;

export const resources = {
	"en-US": { translation: english },
	"zh-CN": { translation: simplifiedChinese },
} as const;

function localStorageOrNull() {
	if (typeof window === "undefined") return null;
	try {
		return window.localStorage;
	} catch {
		return null;
	}
}

function isInterfaceLocalePreference(
	value: string | null,
): value is InterfaceLocalePreference {
	return (
		value === "system" || RESOURCE_LOCALES.includes(value as ResourceLocale)
	);
}

export function getSystemLanguage() {
	if (typeof navigator === "undefined") return "en-US";
	return navigator.languages?.[0] ?? navigator.language ?? "en-US";
}

export function resolveSystemLocale(
	language = getSystemLanguage(),
): ResourceLocale {
	return /^zh(?:-|$)/i.test(language.trim()) ? "zh-CN" : "en-US";
}

export function resolveInterfaceLocale(
	preference: InterfaceLocalePreference,
	systemLanguage = getSystemLanguage(),
): ResourceLocale {
	return preference === "system"
		? resolveSystemLocale(systemLanguage)
		: preference;
}

export function getInterfaceLocalePreference(
	storage: Storage | null = localStorageOrNull(),
): InterfaceLocalePreference {
	try {
		const stored = storage?.getItem(INTERFACE_LOCALE_STORAGE_KEY) ?? null;
		return isInterfaceLocalePreference(stored) ? stored : "system";
	} catch {
		return "system";
	}
}

export function setInterfaceLocalePreference(
	preference: InterfaceLocalePreference,
	storage: Storage | null = localStorageOrNull(),
) {
	try {
		storage?.setItem(
			INTERFACE_LOCALE_STORAGE_KEY,
			isInterfaceLocalePreference(preference) ? preference : "system",
		);
	} catch {
		// Device storage is optional; presentation still changes in memory.
	}
}

export async function changeInterfaceLocale(
	preference: InterfaceLocalePreference,
	storage: Storage | null = localStorageOrNull(),
) {
	setInterfaceLocalePreference(preference, storage);
	await i18n.changeLanguage(resolveInterfaceLocale(preference));
}

function asResourceLocale(value?: string | null): ResourceLocale {
	return value && /^zh(?:-|$)/i.test(value) ? "zh-CN" : "en-US";
}

export function getActiveLocale(): ResourceLocale {
	return asResourceLocale(i18n.resolvedLanguage ?? i18n.language);
}

export function formatDateTime(
	value: Date | number | string,
	options?: Intl.DateTimeFormatOptions,
) {
	const date = value instanceof Date ? value : new Date(value);
	return new Intl.DateTimeFormat(getActiveLocale(), options).format(date);
}

export function formatNumber(
	value: number | bigint,
	options?: Intl.NumberFormatOptions,
) {
	return new Intl.NumberFormat(getActiveLocale(), options).format(value);
}

export function formatDecimal(
	value: string,
	options: Intl.NumberFormatOptions = {},
) {
	if (!/^-?\d+(?:\.\d+)?$/.test(value)) return value;

	const negative = value.startsWith("-");
	const [integer, fraction] = value.replace(/^-/, "").split(".");
	const maximumFractionDigits = options.maximumFractionDigits;
	let displayInteger = integer;
	let displayFraction: string | undefined = fraction;
	if (
		fraction !== undefined &&
		maximumFractionDigits !== undefined &&
		fraction.length > maximumFractionDigits
	) {
		const digits = Math.max(0, Math.floor(maximumFractionDigits));
		const scale = 10n ** BigInt(digits);
		let scaled =
			BigInt(integer) * scale +
			BigInt(fraction.slice(0, digits).padEnd(digits, "0") || "0");
		if (fraction[digits] >= "5") scaled += 1n;
		displayInteger = (scaled / scale).toString();
		displayFraction =
			digits === 0
				? undefined
				: (scaled % scale).toString().padStart(digits, "0").replace(/0+$/, "");
	}
	const separators = new Intl.NumberFormat(getActiveLocale()).formatToParts(
		1234567.89,
	);
	const group = separators.find((part) => part.type === "group")?.value ?? ",";
	const decimal =
		separators.find((part) => part.type === "decimal")?.value ?? ".";
	const groupedInteger =
		options.useGrouping === false
			? displayInteger
			: displayInteger.replace(/\B(?=(\d{3})+(?!\d))/g, () => group);

	return `${negative ? "-" : ""}${groupedInteger}${displayFraction ? decimal + displayFraction : ""}`;
}

export const i18n = i18next;
const initialLocale = resolveInterfaceLocale(getInterfaceLocalePreference());

function updateDocumentLanguage(locale: string) {
	if (typeof document !== "undefined") {
		document.documentElement.lang = asResourceLocale(locale);
	}
}

updateDocumentLanguage(initialLocale);
i18n.on("languageChanged", updateDocumentLanguage);

if (!i18n.isInitialized) {
	void i18n.use(initReactI18next).init({
		resources,
		lng: initialLocale,
		fallbackLng: "en-US",
		supportedLngs: [...RESOURCE_LOCALES],
		load: "currentOnly",
		defaultNS: "translation",
		initAsync: false,
		returnEmptyString: false,
		interpolation: { escapeValue: false },
	});
}
