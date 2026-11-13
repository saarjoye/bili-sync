<script lang="ts">
	import { onMount } from 'svelte';
	import { Card, CardContent, CardHeader, CardTitle } from '$lib/components/ui/card/index.js';
	import { Progress } from '$lib/components/ui/progress/index.js';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { Label } from '$lib/components/ui/label/index.js';
	import { Switch } from '$lib/components/ui/switch/index.js';
	import * as Chart from '$lib/components/ui/chart/index.js';
	import MyChartTooltip from '$lib/components/custom/my-chart-tooltip.svelte';
	import { curveNatural } from 'd3-shape';
	import { BarChart, AreaChart } from 'layerchart';
	import { setBreadcrumb } from '$lib/stores/breadcrumb';
	import { toast } from 'svelte-sonner';
	import CloudDownloadIcon from '@lucide/svelte/icons/cloud-download';
	import api from '$lib/api';
	import type {
		DashBoardResponse,
		DirUpload115ConfigResponse,
		DirUpload115ConfigUpdate,
		SysInfo,
		ApiError,
		TaskStatus,
		UploadHourly,
		UploadStatus
	} from '$lib/types';
	import CalendarIcon from '@lucide/svelte/icons/calendar';
	import CircleCheckBigIcon from '@lucide/svelte/icons/circle-check-big';
	import ClockIcon from '@lucide/svelte/icons/clock';
	import CpuIcon from '@lucide/svelte/icons/cpu';
	import DatabaseIcon from '@lucide/svelte/icons/database';
	import DownloadIcon from '@lucide/svelte/icons/download';
	import FolderIcon from '@lucide/svelte/icons/folder';
	import HardDriveIcon from '@lucide/svelte/icons/hard-drive';
	import HeartIcon from '@lucide/svelte/icons/heart';
	import MemoryStickIcon from '@lucide/svelte/icons/memory-stick';
	import PlayIcon from '@lucide/svelte/icons/play';
	import UserIcon from '@lucide/svelte/icons/user';
	import VideoIcon from '@lucide/svelte/icons/video';

	let dashboardData = $state<DashBoardResponse | null>(null);
	let sysInfo = $state<SysInfo | null>(null);
	let taskStatus = $state<TaskStatus | null>(null);
	let uploadStatus = $state<UploadStatus | null>(null);

	// 由上传状态派生的视图数据
	let queue = $derived(uploadStatus?.queue);
	let round = $derived(uploadStatus?.round);
	let totalVideos = $derived((dashboardData?.videos_by_day ?? []).reduce((s, d) => s + d.cnt, 0));

	// 口径修正：delete 模式下文件上传后即被删除，queue.done 恒为 0，统计统一改用 totals
	let totalUploaded = $derived(uploadStatus?.totals?.uploaded ?? 0);
	let totalDeleted = $derived(uploadStatus?.totals?.deleted ?? 0);
	let pendingCount = $derived(queue?.pending ?? 0);
	let waitingCount = $derived(queue?.waiting ?? 0);
	let failedCount = $derived(queue?.failed ?? 0);
	let backlog = $derived(pendingCount + waitingCount);
	let completionPct = $derived(
		totalUploaded + backlog > 0
			? Math.round((totalUploaded / (totalUploaded + backlog)) * 1000) / 10
			: 0
	);
	let completionTotal = $derived(totalUploaded + backlog);

	// 节奏参数
	let maxPerRound = $derived(uploadStatus?.max_per_round ?? 0);
	let capPerHour = $derived(uploadStatus?.max_per_hour ?? 0);
	let uploadedLastHour = $derived(uploadStatus?.uploaded_last_hour ?? 0);
	let intervalMinutes = $derived(
		uploadStatus?.interval_seconds ? Math.round(uploadStatus.interval_seconds / 60) : 0
	);
	let intervalHours = $derived(intervalMinutes > 0 ? intervalMinutes / 60 : 0);
	// 友好显示：90 分钟以内用分钟，否则用小时
	let intervalLabel = $derived(
		intervalMinutes <= 0
			? '-'
			: intervalMinutes < 90
				? `${intervalMinutes} 分钟`
				: `${Math.round(intervalMinutes / 60)} 小时`
	);

	// ETA：待上传集数 ÷ 单轮上限 × 轮周期
	let etaRounds = $derived(pendingCount > 0 && maxPerRound > 0 ? Math.ceil(pendingCount / maxPerRound) : 0);
	let etaHours = $derived(etaRounds * (intervalHours > 0 ? intervalHours : 6));
	let etaText = $derived(
		etaHours <= 0
			? '-'
			: etaHours < 1
				? `${Math.max(1, Math.round(etaHours * 60))} 分钟`
				: etaHours > 48
					? `${Math.round(etaHours / 24)} 天`
					: `${Math.round(etaHours)} 小时`
	);
	let hourPacePct = $derived(
		capPerHour > 0 ? Math.min(100, Math.round((uploadedLastHour / capPerHour) * 100)) : 0
	);

	// ===== 目录分布 / 滞留 =====
	let dirStats = $derived(uploadStatus?.by_dir ?? []);
	let stuckList = $derived(uploadStatus?.stuck ?? []);

	function segPct(part: number, total: number): number {
		if (total <= 0) return 0;
		return Math.round((part / total) * 1000) / 10;
	}

	// ===== 四段流水线 =====
	let pipelineStages = $derived([
		{
			name: '本地库存（待处理）',
			value: queue?.total ?? 0,
			sub: dirStats.length > 0 ? `${dirStats.length} 个目录` : '—',
			from: '#5EE6C0',
			to: '#2ED3A8'
		},
		{
			name: '待上传 115',
			value: pendingCount,
			sub: `本小时已传 ${uploadedLastHour} / ${capPerHour > 0 ? capPerHour : '-'}`,
			from: '#7FB6FF',
			to: '#4C8DFF'
		},
		{
			name: '等 strm 就绪',
			value: waitingCount,
			sub: `滞留 ${stuckList.length} 个`,
			from: '#8FA0FF',
			to: '#6C7BFF'
		},
		{
			name: '累计已上传',
			value: totalUploaded,
			sub: `已清理本地 ${totalDeleted}`,
			from: '#FFC97A',
			to: '#FFB84D'
		}
	]);

	// ===== 近 12 小时上传量（固定 12 个小时槽，补齐空档；插件 hour_key 为 MM-DD HH:00）=====
	function hourlySlots(
		hourly: UploadHourly[] | undefined
	): { key: string; hour: string; count: number; isNow: boolean }[] {
		const map = new Map<string, number>(
			(hourly ?? []).map((h) => [h.hour, h.count] as [string, number])
		);
		const now = new Date();
		const out: { key: string; hour: string; count: number; isNow: boolean }[] = [];
		const pad = (n: number) => String(n).padStart(2, '0');
		for (let i = 11; i >= 0; i--) {
			const t = new Date(now.getTime() - i * 3600000);
			const key = `${pad(t.getMonth() + 1)}-${pad(t.getDate())} ${pad(t.getHours())}:00`;
			out.push({ key, hour: pad(t.getHours()), count: map.get(key) ?? 0, isNow: i === 0 });
		}
		return out;
	}

	let hourly12 = $derived(hourlySlots(uploadStatus?.hourly));
	let hourlyHasData = $derived(hourly12.some((s) => s.count > 0));
	// 柱高归一基准：本组最大值与每小时上限取大，再留 15% 余量，保证上限虚线不贴顶
	let hourlyMaxBand = $derived(
		Math.max(1, Math.max(hourly12.reduce((m, s) => Math.max(m, s.count), 0), capPerHour)) * 1.15
	);

	function slotBarPct(count: number): number {
		if (count <= 0) return 0;
		return Math.max(3, Math.round((count / hourlyMaxBand) * 100));
	}

	function slotCapPct(): number {
		if (capPerHour <= 0) return 0;
		return Math.min(100, Math.round((capPerHour / hourlyMaxBand) * 100));
	}

	// ===== 健康提示 =====
	let healthIssue = $derived(
		[
			failedCount > 0 ? `${failedCount} 集上传失败` : '',
			stuckList.length > 0 ? `${stuckList.length} 个文件等 strm 超 30 分钟` : ''
		]
			.filter(Boolean)
			.join(' / ')
	);

	// 插件写的 next_run_at 形如 "2026-11-12 22:25:24"：**写状态那一刻 + interval**，每轮都会重算，不是固定值
	function parseStamp(value: string | undefined): Date | null {
		if (!value) return null;
		const m = /^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2})/.exec(value);
		if (!m) return null;
		return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]), Number(m[4]), Number(m[5]));
	}

	const WEEK_LABEL = ['周日', '周一', '周二', '周三', '周四', '周五', '周六'];

	// 绝对时间：今天/明天/后天 HH:mm，更远的写 M-D 周几 HH:mm；没有值写「未排期」
	function fmtClock(d: Date | null): string {
		if (!d) return '未排期';
		const now = new Date();
		const pad = (n: number) => String(n).padStart(2, '0');
		const hm = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
		const startOfDay = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
		const dayDiff = Math.round((startOfDay(d) - startOfDay(now)) / 86400000);
		if (dayDiff === 0) return `今天 ${hm}`;
		if (dayDiff === 1) return `明天 ${hm}`;
		if (dayDiff === 2) return `后天 ${hm}`;
		return `${d.getMonth() + 1}-${d.getDate()} ${WEEK_LABEL[d.getDay()]} ${hm}`;
	}

	// 每秒滴答（只驱动倒计时文本，不影响其它数据）
	let clock = $state(Date.now());

	// 倒计时：HH:MM:SS，到期后前缀 + 表示已过
	function fmtCountdown(d: Date | null, nowMs: number): string {
		if (!d) return '';
		const diff = Math.round((d.getTime() - nowMs) / 1000);
		const abs = Math.abs(diff);
		const pad = (n: number) => String(n).padStart(2, '0');
		const body = `${pad(Math.floor(abs / 3600))}:${pad(Math.floor((abs % 3600) / 60))}:${pad(abs % 60)}`;
		return diff >= 0 ? body : `+${body}`;
	}

	let intervalSeconds = $derived(uploadStatus?.interval_seconds ?? 0);
	let statusUpdatedAt = $derived(parseStamp(uploadStatus?.updated_at));
	let configSavedAt = $derived(parseStamp(uploadStatus?.config_saved_at));
	// 保存插件配置时 MP 会重注册插件的 interval 定时服务 → 下次上传 ≈ 保存配置时刻 + 新周期；
	// 插件下一轮写 status.json 后会清掉 config_saved_at，此时回落到插件自己算的 next_run_at。
	let savedAfterStatus = $derived(
		configSavedAt !== null &&
			(statusUpdatedAt === null || configSavedAt.getTime() > statusUpdatedAt.getTime())
	);
	let pluginNextAt = $derived(parseStamp(uploadStatus?.next_run_at));
	// 1.5.6 起插件被跳过（风控暂停/上一轮没跑完）也会刷新 status.json，这里据此区分三种状态
	let riskPausedUntil = $derived(parseStamp(uploadStatus?.risk_paused_until));
	let pausedReason = $derived(uploadStatus?.paused_reason ?? '');
	let riskPaused = $derived(riskPausedUntil !== null && riskPausedUntil.getTime() > clock);
	let nextUploadAt = $derived(
		savedAfterStatus && intervalSeconds > 0 && configSavedAt
			? new Date(configSavedAt.getTime() + intervalSeconds * 1000)
			: pluginNextAt
	);
	let nextUploadOverdue = $derived(
		!riskPaused && nextUploadAt !== null && nextUploadAt.getTime() < clock
	);
	let nextUploadText = $derived(
		riskPaused
			? `暂停至 ${riskPausedUntil ? fmtClock(riskPausedUntil).replace(/^今天 /, '') : '-'}`
			: nextUploadOverdue
				? '等待触发'
				: fmtClock(nextUploadAt)
	);
	let nextUploadCountdown = $derived(
		riskPaused ? fmtCountdown(riskPausedUntil, clock) : fmtCountdown(nextUploadAt, clock)
	);
	let nextUploadCountdownLabel = $derived(
		riskPaused ? '恢复' : nextUploadOverdue ? '已逾期' : '还剩'
	);
	let nextUploadHint = $derived(
		riskPaused
			? pausedReason || '115 风控冷却中，暂停上传'
			: nextUploadOverdue
				? `已到上一轮预计时间，等待 MoviePilot 调度触发（上次运行 ${shortClock(statusUpdatedAt)}）`
				: savedAfterStatus
					? '按「保存配置时刻 + 周期」估算，下一轮跑完转为插件实测值'
					: `上次运行 ${shortClock(statusUpdatedAt)} · 插件每 ${intervalLabel} 自动一轮`
	);
	let pluginNextText = $derived(fmtClock(pluginNextAt));

	// 只取 HH:mm（给「上次运行」这类提示用）
	function shortClock(d: Date | null): string {
		if (!d) return '-';
		const pad = (n: number) => String(n).padStart(2, '0');
		return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
	}

	// 下载任务的下次时间（taskStatus.next_run 是时间戳/ISO 字符串）
	let nextDownloadAt = $derived(taskStatus?.next_run ? new Date(taskStatus.next_run) : null);
	let nextDownloadText = $derived(fmtClock(nextDownloadAt));
	let nextDownloadCountdown = $derived(fmtCountdown(nextDownloadAt, clock));

	function hhmmss(value: string | undefined): string {
		if (!value) return '-';
		return value.length >= 8 ? value.slice(-8) : value;
	}

	let diskTotal = $derived(sysInfo?.total_disk ?? 0);
	let diskUsed = $derived(sysInfo?.used_disk ?? 0);
	let diskPct = $derived(diskTotal > 0 ? Math.round((diskUsed / diskTotal) * 100) : 0);

	async function loadUploadStatus() {
		try {
			uploadStatus = (await api.getUploadStatus()).data;
		} catch {
			uploadStatus = null;
		}
	}

	// ===== 调度参数（MoviePilot 插件 DirUpload115 配置，由 bili-sync 后端代理）=====
	let mpConfig = $state<DirUpload115ConfigResponse | null>(null);
	let configLoading = $state(false);
	let savingConfig = $state(false);
	let cfgMaxPerRound = $state<number>(20);
	let cfgMaxPerHour = $state<number>(60);
	let cfgIntervalMinutes = $state<number>(360);
	let cfgYieldToMp = $state<boolean>(true);
	let cfgDeleteMode = $state<string>('delete');

	let configAvailable = $derived(mpConfig?.available ?? false);
	let configReason = $derived(mpConfig?.reason ?? '');

	async function loadDirUpload115Config() {
		configLoading = true;
		try {
			const response = await api.getDirUpload115Config();
			const data = response.data;
			mpConfig = data;
			const cfg = (data?.config ?? {}) as Record<string, unknown>;
			const perRound = Number(cfg['max_per_round']);
			if (Number.isFinite(perRound) && perRound > 0) cfgMaxPerRound = perRound;
			const perHour = Number(cfg['max_per_hour']);
			if (Number.isFinite(perHour) && perHour > 0) cfgMaxPerHour = perHour;
			const intervalSeconds = Number(cfg['interval_seconds']);
			if (Number.isFinite(intervalSeconds) && intervalSeconds > 0) {
				cfgIntervalMinutes = Math.max(1, Math.round(intervalSeconds / 60));
			}
			const yieldToMp = cfg['yield_to_mp'];
			if (typeof yieldToMp === 'boolean') cfgYieldToMp = yieldToMp;
			const deleteMode = cfg['delete_mode'];
			if (typeof deleteMode === 'string' && ['delete', 'recycle', 'keep'].includes(deleteMode)) {
				cfgDeleteMode = deleteMode;
			}
		} catch (error) {
			console.error('加载调度参数失败：', error);
			mpConfig = { available: false, reason: (error as ApiError).message };
		} finally {
			configLoading = false;
		}
	}

	async function handleSaveConfig() {
		if (!configAvailable || savingConfig) return;
		savingConfig = true;
		try {
			const body: DirUpload115ConfigUpdate = {
				max_per_round: Math.round(Number(cfgMaxPerRound)),
				max_per_hour: Math.round(Number(cfgMaxPerHour)),
				interval_seconds: Math.round(Number(cfgIntervalMinutes) * 60),
				yield_to_mp: Boolean(cfgYieldToMp),
				delete_mode: cfgDeleteMode
			};
			await api.saveDirUpload115Config(body);
			toast.success('调度参数已保存到 MoviePilot');
			// 让上方卡片立刻同步：先刷 status.json，再重拉插件配置
			await loadUploadStatus();
			await loadDirUpload115Config();
		} catch (error) {
			console.error('保存调度参数失败：', error);
			toast.error('保存调度参数失败', {
				description: (error as ApiError).message
			});
		} finally {
			savingConfig = false;
		}
	}

	let loading = $state(false);
	let triggering = $state(false);
	let memoryHistory = $state<Array<{ time: number; used: number; process: number }>>([]);
	let cpuHistory = $state<Array<{ time: number; used: number; process: number }>>([]);
	let unsubscribeSysInfo: (() => void) | null = null;
	let unsubscribeTasks: (() => void) | null = null;

	function formatBytes(bytes: number): string {
		if (bytes === 0) return '0 B';
		const k = 1024;
		const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
		const i = Math.floor(Math.log(bytes) / Math.log(k));
		return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
	}

	function formatCpu(cpu: number): string {
		return `${cpu.toFixed(1)}%`;
	}

	function formatTimestamp(timestamp: number): string {
		return new Date(timestamp).toLocaleString('en-US', {
			hour: '2-digit',
			minute: '2-digit',
			second: '2-digit',
			hour12: true
		});
	}

	async function loadDashboard() {
		loading = true;
		try {
			const response = await api.getDashboard();
			dashboardData = response.data;
		} catch (error) {
			console.error('加载仪表盘数据失败：', error);
			toast.error('加载仪表盘数据失败', {
				description: (error as ApiError).message
			});
		} finally {
			loading = false;
		}
	}

	async function handleTriggerDownload() {
		triggering = true;
		try {
			await api.triggerDownloadTask();
			toast.success('已触发下载任务', {
				description: '任务将立即开始执行'
			});
		} catch (error) {
			console.error('触发下载任务失败：', error);
			toast.error('触发下载任务失败', {
				description: (error as ApiError).message
			});
		} finally {
			triggering = false;
		}
	}

	const videoChartConfig = {
		videos: {
			label: '视频数量',
			color: 'var(--primary)'
		}
	} satisfies Chart.ChartConfig;

	const memoryChartConfig = {
		used: {
			label: '整体占用',
			color: 'var(--primary)'
		},
		process: {
			label: '程序占用',
			color: 'oklch(from var(--primary) calc(l * 0.6) c h)'
		}
	} satisfies Chart.ChartConfig;

	const cpuChartConfig = {
		used: {
			label: '整体占用',
			color: 'var(--primary)'
		},
		process: {
			label: '程序占用',
			color: 'oklch(from var(--primary) calc(l * 0.6) c h)'
		}
	} satisfies Chart.ChartConfig;

	function pushSysInfo(data: SysInfo) {
		memoryHistory = [
			...memoryHistory.slice(-14),
			{
				time: data.timestamp,
				used: data.used_memory,
				process: data.process_memory
			}
		];
		cpuHistory = [
			...cpuHistory.slice(-14),
			{
				time: data.timestamp,
				used: data.used_cpu,
				process: data.process_cpu
			}
		];
	}

	const diskUsagePercent = $derived(
		sysInfo ? ((sysInfo.total_disk - sysInfo.available_disk) / sysInfo.total_disk) * 100 : 0
	);

	onMount(() => {
		setBreadcrumb([{ label: '仪表盘' }]);

		unsubscribeSysInfo = api.subscribeToSysInfo((data) => {
			sysInfo = data;
			pushSysInfo(data);
		});
		unsubscribeTasks = api.subscribeToTasks((data: TaskStatus) => {
			taskStatus = data;
		});
		loadDashboard();
		return () => {
			if (unsubscribeSysInfo) {
				unsubscribeSysInfo();
				unsubscribeSysInfo = null;
			}
			if (unsubscribeTasks) {
				unsubscribeTasks();
				unsubscribeTasks = null;
			}
		};
	});

	onMount(() => {
		loadUploadStatus();
		loadDirUpload115Config();
		const timer = setInterval(loadUploadStatus, 60000);
		// 每秒滴答：驱动「下次上传 / 下次下载」的倒计时
		const clockTimer = setInterval(() => (clock = Date.now()), 1000);
		return () => {
			clearInterval(timer);
			clearInterval(clockTimer);
		};
	});
</script>
<div class="min-h-screen bg-gradient-to-br from-[#EAF7F4] via-[#EFF6FB] to-[#E8F1FB] p-4 md:p-6">

	<!-- ========== KPI 行 ========== -->
	<div class="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
		{#each [
			{ label: '订阅总数', value: (dashboardData?.enabled_collections ?? 0) + (dashboardData?.enabled_submissions ?? 0) + (dashboardData?.enabled_favorites ?? 0), sub: `合集 ${dashboardData?.enabled_collections ?? 0} · 投稿 ${dashboardData?.enabled_submissions ?? 0} · 收藏 ${dashboardData?.enabled_favorites ?? 0}`, color: 'from-emerald-400 to-teal-500', icon: VideoIcon },
			{ label: '近 7 日入库', value: totalVideos, sub: '视频', color: 'from-sky-400 to-blue-500', icon: CloudDownloadIcon },
			{ label: '待上传 115', value: pendingCount, sub: `共 ${queue?.total ?? 0} 集`, color: 'from-violet-400 to-indigo-500', icon: DownloadIcon },
			{ label: '已上传', value: totalUploaded, sub: `${completionPct}%`, color: 'from-emerald-400 to-green-500', icon: CircleCheckBigIcon },
			{ label: '等 strm', value: waitingCount, sub: '未就绪', color: 'from-amber-400 to-orange-500', icon: ClockIcon },
			{ label: '失败', value: failedCount, sub: '需关注', color: 'from-rose-400 to-red-500', icon: DatabaseIcon }
		] as kpi}
			{@const Icon = kpi.icon}
			<div
				class="group relative overflow-hidden rounded-2xl border border-white/70 bg-white/75 p-4 shadow-[0_8px_24px_rgba(31,79,120,0.08)] backdrop-blur"
			>
				<div class="flex items-start justify-between">
					<div>
						<p class="text-muted-foreground text-xs tracking-wide">{kpi.label}</p>
						<p class="mt-1 text-2xl font-bold tabular-nums md:text-3xl">{kpi.value}</p>
						<p class="text-muted-foreground mt-1 text-[11px]">{kpi.sub}</p>
					</div>
					<div
						class="rounded-xl bg-gradient-to-br {kpi.color} p-2 text-white opacity-90 shadow-sm transition group-hover:opacity-100"
					>
						<Icon class="h-4 w-4" />
					</div>
				</div>
			</div>
		{/each}
	</div>

	<!-- ========== 流水线健康 + 任务状态 ========== -->
	<div class="mt-4 grid grid-cols-1 gap-4 lg:grid-cols-3">
		<!-- 流水线健康 -->
		<div class="rounded-2xl border border-white/70 bg-white/75 p-5 shadow-[0_8px_24px_rgba(31,79,120,0.08)] backdrop-blur lg:col-span-2">
			<!-- a) 标题 + 节奏胶囊 -->
			<div class="mb-4 flex flex-wrap items-center justify-between gap-2">
				<h3 class="text-sm font-semibold">流水线健康</h3>
				<div class="flex flex-wrap items-center gap-1.5">
					<span class="rounded-full border border-slate-200 bg-white/85 px-2.5 py-0.5 text-[10px] whitespace-nowrap">
						单轮 {maxPerRound > 0 ? maxPerRound : '-'} 集
					</span>
					<span class="rounded-full border border-slate-200 bg-white/85 px-2.5 py-0.5 text-[10px] whitespace-nowrap">
						周期 {intervalLabel}
					</span>
					<span class="rounded-full border border-slate-200 bg-white/85 px-2.5 py-0.5 text-[10px] whitespace-nowrap">
						小时上限 {capPerHour > 0 ? capPerHour : '-'}
					</span>
				</div>
			</div>

			<!-- b) 四段流水线 -->
			<div class="grid grid-cols-2 gap-3 md:grid-cols-4">
				{#each pipelineStages as seg}
					<div class="relative overflow-hidden rounded-xl bg-white/70 px-3 pt-3 pb-2.5">
						<div
							class="absolute inset-x-0 top-0 h-1"
							style="background: linear-gradient(90deg, {seg.from}, {seg.to});"
						></div>
						<p class="text-[11px] text-slate-600">{seg.name}</p>
						<p class="mt-0.5 text-2xl font-bold tabular-nums">{seg.value}</p>
						<p class="text-[10px] text-slate-400">{seg.sub}</p>
					</div>
				{/each}
			</div>

			<!-- c) 整体完成度 -->
			<div class="mt-3 flex items-center gap-3">
				<span class="text-muted-foreground text-[10px] whitespace-nowrap">整体完成度</span>
				<div class="h-2.5 flex-1 overflow-hidden rounded-full bg-slate-200/70">
					<div
						class="h-full rounded-full bg-gradient-to-r from-emerald-400 to-blue-500 transition-all"
						style="width: {completionPct}%"
					></div>
				</div>
				<span class="text-lg font-bold tabular-nums">{completionPct}%</span>
				<span class="text-muted-foreground text-[11px] whitespace-nowrap">
					{totalUploaded} / {completionTotal} 集已完成
				</span>
			</div>

			<!-- d) 近 12 小时上传量 -->
			<div class="mt-3 border-t border-white/60 pt-3">
				<div class="mb-2 flex items-center justify-between">
					<span class="text-[11px] font-medium">近 12 小时上传量</span>
					<span class="rounded-full bg-rose-50 px-2 py-0.5 text-[10px] text-rose-500">
						上限 {capPerHour > 0 ? capPerHour : '-'}/小时
					</span>
				</div>
				{#if hourlyHasData}
					<div class="relative h-24">
						<div class="flex h-full items-end gap-1">
							{#each hourly12 as s}
								<div
									class="group flex h-full flex-1 flex-col items-center justify-end"
									title="{s.key} · {s.count} 集"
								>
									{#if s.count > 0}
										<div
											class="relative w-full max-w-[42px] rounded-t-md bg-gradient-to-t from-sky-300 to-emerald-400 transition-all"
											style="height: {slotBarPct(s.count)}%"
										>
											<span
												class="absolute -top-5 left-1/2 -translate-x-1/2 rounded-full bg-white/95 px-1.5 py-0.5 text-[10px] font-medium tabular-nums opacity-0 shadow-sm transition group-hover:opacity-100"
											>
												{s.count}
											</span>
										</div>
									{:else}
										<div class="h-0.5 w-full max-w-[42px] rounded-t-sm bg-slate-200/70"></div>
									{/if}
								</div>
							{/each}
						</div>
						{#if capPerHour > 0}
							<div
								class="pointer-events-none absolute inset-x-0 z-10 border-t border-dashed border-rose-400/80"
								style="bottom: {slotCapPct()}%"
							></div>
						{/if}
					</div>
					<div class="mt-1 flex gap-1">
						{#each hourly12 as s}
							<div
								class="flex-1 text-center text-[9px] tabular-nums {s.isNow
									? 'font-medium text-slate-600'
									: 'text-muted-foreground'}"
							>
								{s.hour}
							</div>
						{/each}
					</div>
				{:else}
					<p class="text-muted-foreground py-6 text-center text-[11px]">暂无数据（等待下一轮上传统计）</p>
				{/if}
			</div>

			<!-- e) 三格：预计完成 / 本小时配速 / 失败·滞留 -->
			<div class="mt-3 grid grid-cols-3 gap-2.5">
				<div class="rounded-xl bg-white/70 px-3 py-2.5">
					<p class="text-[10px] text-slate-400">预计完成</p>
					{#if etaRounds > 0 && pendingCount > 0}
						<p class="text-base font-bold tabular-nums">还需 {etaRounds} 轮</p>
						<p class="text-[10px] text-slate-400">≈ {etaText}</p>
					{:else}
						<p class="text-base font-bold tabular-nums">已清空</p>
						<p class="text-[10px] text-slate-400">暂无待上传</p>
					{/if}
				</div>
				<div class="rounded-xl bg-white/70 px-3 py-2.5">
					<p class="text-[10px] text-slate-400">本小时配速</p>
					<p class="text-base font-bold tabular-nums">
						{uploadedLastHour} / {capPerHour > 0 ? capPerHour : '-'} 集
					</p>
					<div class="mt-1 h-1.5 w-full overflow-hidden rounded-full bg-slate-200/70">
						<div
							class="h-full rounded-full bg-gradient-to-r from-emerald-400 to-blue-500 transition-all"
							style="width: {hourPacePct}%"
						></div>
					</div>
				</div>
				<div class="rounded-xl bg-white/70 px-3 py-2.5">
					<p class="text-[10px] text-slate-400">失败 / 滞留</p>
					<p class="text-base font-bold tabular-nums">
						<span class={failedCount > 0 ? 'text-rose-500' : 'text-emerald-600'}>{failedCount}</span>
						<span class="text-muted-foreground text-[11px]"> · 滞留 </span>
						<span class={stuckList.length > 0 ? 'text-amber-600' : 'text-emerald-600'}>{stuckList.length}</span>
					</p>
				</div>
			</div>

			<!-- f) 健康提示条 -->
			{#if failedCount > 0 || stuckList.length > 0}
				<div class="mt-3 rounded-xl bg-amber-50 px-3 py-2 text-[11px] text-amber-700">
					⚠ {healthIssue}
				</div>
			{:else}
				<div class="mt-3 rounded-xl bg-emerald-50 px-3 py-2 text-[11px] text-emerald-700">
					{#if riskPaused}
						<span class="text-amber-700">
							115 风控暂停中：{pausedReason || '冷却尚未结束'}；预计 {nextUploadText}
						</span>
					{:else if nextUploadOverdue}
						<span class="text-rose-600">
							已到预计时间但还没触发（上次运行 {shortClock(statusUpdatedAt)}），正在等 MoviePilot 调度；按当前配速预计还需 {etaRounds} 轮
						</span>
					{:else}
						队列健康：无失败、无滞留；下一轮 {nextUploadText}（还剩 {nextUploadCountdown}）开始，按当前配速预计再传 {etaRounds} 轮
					{/if}
				</div>
			{/if}
		</div>

		<!-- 状态面板 -->
		<div class="rounded-2xl border border-white/70 bg-white/75 p-5 shadow-[0_8px_24px_rgba(31,79,120,0.08)] backdrop-blur">
			<h3 class="mb-3 text-sm font-semibold">任务状态</h3>
			<div class="space-y-3 text-sm">
				<div class="flex items-center justify-between rounded-xl bg-white/70 px-3 py-2">
					<span class="text-muted-foreground text-xs">下载任务</span>
					<Badge variant={taskStatus?.is_running ? 'default' : 'secondary'}>
						{taskStatus?.is_running ? '运行中' : '空闲'}
					</Badge>
				</div>
				<div class="rounded-xl bg-white/70 px-3 py-2">
					<div class="flex items-center justify-between">
						<span class="text-muted-foreground text-xs">下次下载</span>
						<span class="tabular-nums text-xs font-medium whitespace-nowrap">{nextDownloadText}</span>
					</div>
					<div class="mt-1 flex items-center justify-between">
						<span class="text-muted-foreground text-[10px]">bili-sync 下载任务</span>
						<span class="flex items-baseline gap-1">
							<span class="text-muted-foreground text-[10px]">还剩</span>
							<span class="tabular-nums text-base font-bold text-slate-700">{nextDownloadCountdown || '—'}</span>
						</span>
					</div>
				</div>
				<div class="flex items-center justify-between rounded-xl bg-white/70 px-3 py-2">
					<span class="text-muted-foreground text-xs">上传调度</span>
					<Badge variant={uploadStatus?.available ? 'default' : 'secondary'}>
						{uploadStatus?.available ? (uploadStatus?.enabled ? '已启用' : '已暂停') : '未启用'}
					</Badge>
				</div>
				<div class="rounded-xl bg-white/70 px-3 py-2">
					<div class="flex items-center justify-between">
						<span class="text-muted-foreground text-xs">下次上传</span>
						<span
							class="tabular-nums text-xs font-medium whitespace-nowrap"
							title="插件 status.json 记录：{pluginNextText}"
							>{nextUploadText}</span
						>
					</div>
					<div class="mt-1 flex items-center justify-between gap-2">
						<span class="text-muted-foreground flex-1 text-[10px] leading-tight">{nextUploadHint}</span>
						<span class="flex shrink-0 items-baseline gap-1">
							<span class="text-muted-foreground text-[10px]">{nextUploadCountdownLabel}</span>
							<span
								class="tabular-nums text-base font-bold {riskPaused
									? 'text-amber-600'
									: nextUploadOverdue
										? 'text-rose-500'
										: 'text-slate-700'}">{nextUploadCountdown || '—'}</span
							>
						</span>
					</div>
				</div>
				<div class="flex items-center justify-between rounded-xl bg-white/70 px-3 py-2">
					<span class="text-muted-foreground text-xs">数据更新</span>
					<span class="tabular-nums text-xs">{hhmmss(uploadStatus?.updated_at)}</span>
				</div>
				<div class="rounded-xl bg-white/70 px-3 py-2">
					<div class="flex items-center justify-between">
						<span class="text-muted-foreground text-xs">本小时已传</span>
						<span class="tabular-nums text-xs">{uploadedLastHour} / {capPerHour > 0 ? capPerHour : '-'}</span>
					</div>
					<div class="mt-1.5 h-1.5 w-full overflow-hidden rounded-full bg-slate-200/70">
						<div
							class="h-full rounded-full bg-gradient-to-r from-emerald-400 to-blue-500 transition-all"
							style="width: {hourPacePct}%"
						></div>
					</div>
				</div>
			</div>
		</div>
	</div>

	<!-- ========== 三张上传卡 ========== -->
	<div class="mt-4 grid grid-cols-1 gap-4 lg:grid-cols-3">
		<!-- 上传进度 -->
		<div class="rounded-2xl border border-white/70 bg-white/75 p-5 shadow-[0_8px_24px_rgba(31,79,120,0.08)] backdrop-blur">
			<div class="mb-3 flex items-center justify-between">
				<h3 class="text-sm font-semibold">上传到 115</h3>
				<span class="text-muted-foreground text-[11px]">{uploadStatus?.updated_at ?? ''}</span>
			</div>
			{#if !uploadStatus?.available}
				<p class="text-muted-foreground text-xs">{uploadStatus?.reason ?? '插件未启用'}</p>
			{:else}
				<div class="mb-1 flex items-end justify-between">
					<span class="text-3xl font-bold tabular-nums">{pendingCount}</span>
					<span class="text-muted-foreground text-xs">待上传</span>
				</div>
				<div class="h-2.5 w-full overflow-hidden rounded-full bg-slate-200/70">
					<div
						class="h-full rounded-full bg-gradient-to-r from-emerald-400 to-teal-500 transition-all"
						style="width: {completionPct}%"
					></div>
				</div>
				<p class="text-muted-foreground mt-1 text-[11px]">
					已完成 {totalUploaded} / {completionTotal}（{completionPct}%）
				</p>
				<div class="mt-3 grid grid-cols-3 gap-2 text-center">
					<div class="rounded-xl bg-white/70 py-2">
						<p class="text-base font-semibold tabular-nums">{round?.uploaded ?? 0}</p>
						<p class="text-muted-foreground text-[10px]">本轮已传</p>
					</div>
					<div class="rounded-xl bg-white/70 py-2">
						<p class="text-base font-semibold tabular-nums">{waitingCount}</p>
						<p class="text-muted-foreground text-[10px]">等 strm</p>
					</div>
					<div class="rounded-xl bg-white/70 py-2">
						<p class="text-base font-semibold tabular-nums text-rose-500">{failedCount}</p>
						<p class="text-muted-foreground text-[10px]">失败</p>
					</div>
				</div>
			{/if}
		</div>

		<!-- 上传调度 -->
		<div class="rounded-2xl border border-white/70 bg-white/75 p-5 shadow-[0_8px_24px_rgba(31,79,120,0.08)] backdrop-blur">
			<h3 class="mb-3 text-sm font-semibold">上传调度</h3>
			<div class="space-y-2 text-xs">
				<div class="flex items-center justify-between">
					<span class="text-muted-foreground">周期</span>
					<span class="tabular-nums">{intervalLabel}</span>
				</div>
				<div class="flex items-center justify-between">
					<span class="text-muted-foreground">本轮开始</span>
					<span class="tabular-nums">{uploadStatus?.round_started_at ?? '-'}</span>
				</div>
				<div class="flex items-center justify-between">
					<span class="text-muted-foreground">本轮扫描</span>
					<span class="tabular-nums">{round?.scanned ?? 0} 集</span>
				</div>
				<div class="flex items-center justify-between">
					<span class="text-muted-foreground">让路 MP</span>
					<span>{uploadStatus?.yield_to_mp ? '开启' : '关闭'}</span>
				</div>
			</div>
		</div>

		<!-- 最近上传并清理 -->
		<div class="rounded-2xl border border-white/70 bg-white/75 p-5 shadow-[0_8px_24px_rgba(31,79,120,0.08)] backdrop-blur">
			<h3 class="mb-3 text-sm font-semibold">最近上传并清理</h3>
			{#if !uploadStatus?.recent?.length}
				<p class="text-muted-foreground text-xs">暂无记录</p>
			{:else}
				<div class="max-h-64 space-y-1.5 overflow-auto pr-1">
					{#each uploadStatus.recent.slice().reverse() as e}
						<div class="flex items-center gap-2 rounded-lg bg-white/70 px-2 py-1.5">
							<span
								class="h-2 w-2 shrink-0 rounded-full"
								style="background:{e.outcome === 'uploaded'
									? '#2ED3A8'
									: e.outcome === 'waiting'
										? '#4C8DFF'
										: e.outcome === 'failed'
											? '#FF6B6B'
											: '#FFB84D'}"
							></span>
							<span class="text-muted-foreground shrink-0 text-[10px] tabular-nums">{e.time}</span>
							<span class="flex-1 truncate text-[11px]">{e.name}</span>
							<span class="shrink-0 text-[10px]">{e.label}</span>
						</div>
					{/each}
				</div>
			{/if}
		</div>
	</div>

	<!-- ========== 目录分布 + 存储 ========== -->
	<div class="mt-4 grid grid-cols-1 gap-4 lg:grid-cols-3">
		<!-- 目录分布 + 滞留 -->
		<div class="rounded-2xl border border-white/70 bg-white/75 p-5 shadow-[0_8px_24px_rgba(31,79,120,0.08)] backdrop-blur lg:col-span-2">
			<div class="mb-2 flex items-center justify-between">
				<h3 class="text-sm font-semibold">目录分布</h3>
				<span class="text-muted-foreground text-[10px]">{dirStats.length} 个目录</span>
			</div>
			{#if dirStats.length === 0}
				<p class="text-muted-foreground py-4 text-center text-xs">暂无数据（等待下一轮上传统计）</p>
			{:else}
				<div class="max-h-44 space-y-2.5 overflow-auto pr-1">
					{#each dirStats as d}
						<div>
							<div class="mb-1 flex items-center justify-between text-[11px]">
								<span class="flex-1 truncate font-medium">{d.dir}</span>
								<span class="text-muted-foreground ml-2 shrink-0 tabular-nums">
									{d.done}/{d.total}（{d.total > 0 ? Math.round((d.done / d.total) * 100) : 0}%）
								</span>
							</div>
							<div class="flex h-2.5 w-full overflow-hidden rounded-full bg-slate-200/70">
								<div class="h-full" style="width: {segPct(d.done, d.total)}%; background: #2ED3A8"></div>
								<div class="h-full" style="width: {segPct(d.pending, d.total)}%; background: #4C8DFF"></div>
								<div class="h-full" style="width: {segPct(d.waiting, d.total)}%; background: #FFB84D"></div>
							</div>
						</div>
					{/each}
				</div>
				<div class="text-muted-foreground mt-2 flex flex-wrap items-center gap-3 text-[10px]">
					<span class="flex items-center gap-1">
						<span class="h-2 w-2 rounded-full" style="background: #2ED3A8"></span>已完成
					</span>
					<span class="flex items-center gap-1">
						<span class="h-2 w-2 rounded-full" style="background: #4C8DFF"></span>待上传
					</span>
					<span class="flex items-center gap-1">
						<span class="h-2 w-2 rounded-full" style="background: #FFB84D"></span>等 strm
					</span>
				</div>
			{/if}
			<div class="mt-3 rounded-xl bg-white/70 p-2.5">
				<p class="text-[10px] font-medium">滞留</p>
				{#if stuckList.length === 0}
					<p class="text-muted-foreground mt-0.5 text-[11px]">无滞留</p>
				{:else}
					<div class="mt-1.5 flex flex-wrap gap-1.5">
						{#each stuckList as s}
							<span class="rounded-full bg-amber-100 px-2 py-0.5 text-[10px] text-amber-700">
								{s.name} · 已滞留 {s.minutes} 分钟
							</span>
						{/each}
					</div>
				{/if}
			</div>
		</div>

		<div class="rounded-2xl border border-white/70 bg-white/75 p-5 shadow-[0_8px_24px_rgba(31,79,120,0.08)] backdrop-blur">
			<h3 class="mb-3 text-sm font-semibold">存储空间</h3>
			<div class="flex items-center gap-4">
				<svg viewBox="0 0 42 42" class="h-24 w-24 -rotate-90">
					<circle cx="21" cy="21" r="15.9" fill="none" stroke="#E5EAF2" stroke-width="5" />
					<circle
						cx="21"
						cy="21"
						r="15.9"
						fill="none"
						stroke="url(#g1)"
						stroke-width="5"
						stroke-linecap="round"
						stroke-dasharray="{diskPct} {100 - diskPct}"
						pathLength="100"
					/>
					<defs>
						<linearGradient id="g1" x1="0" y1="0" x2="1" y2="1">
							<stop offset="0%" stop-color="#2ED3A8" />
							<stop offset="100%" stop-color="#4C8DFF" />
						</linearGradient>
					</defs>
				</svg>
				<div class="text-sm">
					<p class="text-2xl font-bold tabular-nums">{formatBytes(sysInfo?.available_disk ?? 0)}</p>
					<p class="text-muted-foreground text-[11px]">可用 / 共 {formatBytes(diskTotal)}</p>
					<p class="text-muted-foreground mt-1 text-[11px]">已用 {diskPct}%</p>
				</div>
			</div>
			<Button class="mt-3 w-full" onclick={handleTriggerDownload} disabled={triggering}>
				<PlayIcon class="mr-1 h-4 w-4" />
				{triggering ? '触发中…' : '立即执行下载任务'}
			</Button>
		</div>
	</div>

	<!-- ========== 调度参数（MoviePilot 插件 DirUpload115） ========== -->
	<div class="mt-4 rounded-2xl border border-white/70 bg-white/75 p-5 shadow-[0_8px_24px_rgba(31,79,120,0.08)] backdrop-blur">
		<div class="mb-4 flex flex-wrap items-center justify-between gap-2">
			<div class="flex items-center gap-2">
				<h3 class="text-sm font-semibold">调度参数</h3>
				{#if mpConfig && !configAvailable}
					<Badge variant="secondary">只读</Badge>
				{/if}
			</div>
			<Button variant="ghost" size="sm" onclick={loadDirUpload115Config} disabled={configLoading}>
				{configLoading ? '刷新中…' : '刷新配置'}
			</Button>
		</div>

		{#if mpConfig && !configAvailable}
			<p class="text-muted-foreground mb-4 rounded-xl bg-white/70 px-3 py-2 text-[11px]">
				只读：{configReason || 'MoviePilot 插件配置不可用'}
			</p>
		{/if}

		<div class="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-5">
			<div class="space-y-1.5">
				<Label for="cfg-max-per-round">每轮上限</Label>
				<Input
					id="cfg-max-per-round"
					type="number"
					min="1"
					max="200"
					step="1"
					bind:value={cfgMaxPerRound}
					disabled={!configAvailable || savingConfig}
				/>
				<p class="text-muted-foreground text-[10px]">1–200 集</p>
			</div>

			<div class="space-y-1.5">
				<Label for="cfg-max-per-hour">每小时上限</Label>
				<Input
					id="cfg-max-per-hour"
					type="number"
					min="1"
					max="2000"
					step="1"
					bind:value={cfgMaxPerHour}
					disabled={!configAvailable || savingConfig}
				/>
				<p class="text-muted-foreground text-[10px]">1–2000 集</p>
			</div>

			<div class="space-y-1.5">
				<Label for="cfg-interval">周期（分钟）</Label>
				<Input
					id="cfg-interval"
					type="number"
					min="10"
					max="1440"
					step="5"
					bind:value={cfgIntervalMinutes}
					disabled={!configAvailable || savingConfig}
				/>
				<p class="text-muted-foreground text-[10px]">
					单位分钟 · 保存时换算 {Math.round(Number(cfgIntervalMinutes) * 60)} 秒（600–86400，即 10 分钟–24 小时）
				</p>
			</div>

			<div class="space-y-1.5">
				<Label for="cfg-yield-to-mp">让路 MP</Label>
				<div class="flex h-9 items-center gap-2 rounded-md bg-white/70 px-3">
					<Switch
						id="cfg-yield-to-mp"
						bind:checked={cfgYieldToMp}
						disabled={!configAvailable || savingConfig}
					/>
					<span class="text-xs">{cfgYieldToMp ? '开启' : '关闭'}</span>
				</div>
				<p class="text-muted-foreground text-[10px]">MoviePilot 忙碌时暂停上传</p>
			</div>

			<div class="space-y-1.5">
				<Label for="cfg-delete-mode">删除策略</Label>
				<select
					id="cfg-delete-mode"
					class="border-input bg-background h-9 w-full rounded-md border px-3 text-sm shadow-xs outline-none disabled:cursor-not-allowed disabled:opacity-50"
					bind:value={cfgDeleteMode}
					disabled={!configAvailable || savingConfig}
				>
					<option value="delete">就绪后删除</option>
					<option value="recycle">移入回收站</option>
					<option value="keep">保留</option>
				</select>
				<p class="text-muted-foreground text-[10px]">上传/strm 就绪后如何处理本地文件</p>
			</div>
		</div>

		<div class="mt-4 flex flex-wrap items-end justify-between gap-3 border-t border-white/60 pt-3">
			<div class="text-muted-foreground space-y-0.5 text-[10px]">
				<p>下次运行：{uploadStatus?.next_run_at ?? '-'}</p>
				<p>数据来自 MoviePilot 插件 DirUpload115</p>
			</div>
			<Button onclick={handleSaveConfig} disabled={!configAvailable || savingConfig || configLoading}>
				{savingConfig ? '保存中…' : '保存'}
			</Button>
		</div>
	</div>
</div>
