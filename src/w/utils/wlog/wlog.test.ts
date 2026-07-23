/// <reference types="jest" />
import { LogLevel, type WLogConfig, wlog } from "./wlog";

describe("wlog", () => {
	const mockConsole = {
		log: jest.spyOn(console, "log").mockImplementation(() => {}),
		info: jest.spyOn(console, "info").mockImplementation(() => {}),
		warn: jest.spyOn(console, "warn").mockImplementation(() => {}),
		error: jest.spyOn(console, "error").mockImplementation(() => {}),
		debug: jest.spyOn(console, "debug").mockImplementation(() => {}),
		trace: jest.spyOn(console, "trace").mockImplementation(() => {}),
		time: jest.spyOn(console, "time").mockImplementation(() => {}),
		timeEnd: jest.spyOn(console, "timeEnd").mockImplementation(() => {}),
		table: jest.spyOn(console, "table").mockImplementation(() => {}),
		groupCollapsed: jest.spyOn(console, "groupCollapsed").mockImplementation(() => {}),
		groupEnd: jest.spyOn(console, "groupEnd").mockImplementation(() => {}),
		clear: jest.spyOn(console, "clear").mockImplementation(() => {}),
		count: jest.spyOn(console, "count").mockImplementation(() => {}),
		countReset: jest.spyOn(console, "countReset").mockImplementation(() => {}),
		assert: jest.spyOn(console, "assert").mockImplementation(() => {}),
	};

	beforeEach(() => {
		jest.clearAllMocks();
		wlog.reset();
		wlog.setSwitch(true);
		wlog.setLevel(LogLevel.ALL);
	});

	afterAll(() => {
		jest.restoreAllMocks();
	});

	it("should be a defined singleton function", () => {
		expect(wlog).toBeDefined();
		expect(typeof wlog).toBe("function");
	});

	it("should support basic logging", () => {
		wlog("Test Message");
		expect(mockConsole.log).toHaveBeenCalled();
	});

	it("should support label + data pattern", () => {
		wlog("User", { name: "John", age: 30 });
		expect(mockConsole.groupCollapsed).toHaveBeenCalled();
		expect(mockConsole.groupEnd).toHaveBeenCalled();
	});

	it("should support level methods", () => {
		wlog.info("Info message");
		wlog.warn("Warn message");
		wlog.error("Error message");
		wlog.debug("Debug message");
		wlog.trace("Trace message");
		wlog.success("Success message");

		expect(mockConsole.info).toHaveBeenCalled();
		expect(mockConsole.warn).toHaveBeenCalled();
		expect(mockConsole.error).toHaveBeenCalled();
		expect(mockConsole.log).toHaveBeenCalled();
	});

	it("should respect log level filtering", () => {
		wlog.setLevel(LogLevel.WARN);

		wlog.debug("Debug should be hidden");
		wlog.info("Info should be hidden");
		wlog.warn("Warn should be shown");
		wlog.error("Error should be shown");

		expect(mockConsole.debug).not.toHaveBeenCalled();
		expect(mockConsole.info).not.toHaveBeenCalled();
		expect(mockConsole.warn).toHaveBeenCalled();
		expect(mockConsole.error).toHaveBeenCalled();
	});

	it("should disable output when setSwitch is false", () => {
		wlog.setSwitch(false);

		wlog("Should not log");
		wlog.info("Should not log info");
		wlog.error("Should not log error");

		expect(mockConsole.log).not.toHaveBeenCalled();
		expect(mockConsole.info).not.toHaveBeenCalled();
		expect(mockConsole.error).not.toHaveBeenCalled();
	});

	it("should support configuration management", () => {
		const newConfig: Partial<WLogConfig> = {
			prefix: "TEST_APP",
			showTime: false,
			showCaller: false,
			level: LogLevel.DEBUG,
		};

		wlog.config(newConfig);

		const currentConfig = wlog.getConfig();
		expect(currentConfig.prefix).toBe("TEST_APP");
		expect(currentConfig.showTime).toBe(false);
		expect(currentConfig.showCaller).toBe(false);
		expect(currentConfig.level).toBe(LogLevel.DEBUG);
	});

	it("should support setPrefix", () => {
		wlog.setPrefix("MY_MODULE");
		expect(wlog.getConfig().prefix).toBe("MY_MODULE");
	});

	it("should support table output", () => {
		const data = [
			{ id: 1, name: "Alice" },
			{ id: 2, name: "Bob" },
		];

		wlog.table("Users Table", data);
		expect(mockConsole.groupCollapsed).toHaveBeenCalled();
		expect(mockConsole.table).toHaveBeenCalledWith(data);
		expect(mockConsole.groupEnd).toHaveBeenCalled();
	});

	it("should support section and divider formatting", () => {
		wlog.section("Main Section");
		wlog.divider("─", 40);

		expect(mockConsole.log).toHaveBeenCalled();
	});

	it("should support formatted json logging", () => {
		const data = { name: "Test", value: 999 };
		wlog.json("JSON Data", data, 2);

		expect(mockConsole.log).toHaveBeenCalled();
	});

	it("should support timer and counter utility methods", () => {
		wlog.time("TimerLabel");
		wlog.timeEnd("TimerLabel");
		wlog.count("CounterLabel");
		wlog.countReset("CounterLabel");
		wlog.clear();

		expect(mockConsole.time).toHaveBeenCalledWith("TimerLabel");
		expect(mockConsole.timeEnd).toHaveBeenCalledWith("TimerLabel");
		expect(mockConsole.count).toHaveBeenCalledWith("CounterLabel");
		expect(mockConsole.countReset).toHaveBeenCalledWith("CounterLabel");
		expect(mockConsole.clear).toHaveBeenCalled();
	});

	it("should support assertion checking", () => {
		wlog.assert(true, "Should pass");
		wlog.assert(false, "Should fail assertion");

		expect(mockConsole.assert).toHaveBeenCalledWith(false, "Should fail assertion");
	});
});
