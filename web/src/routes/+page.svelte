<script lang="ts">
	import { onMount } from 'svelte';
	import { Card, CardContent, CardHeader, CardTitle } from '$lib/components/ui/card/index.js';
	import { Progress } from '$lib/components/ui/progress/index.js';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Chart from '$lib/components/ui/chart/index.js';
	import MyChartTooltip from '$lib/components/custom/my-chart-tooltip.svelte';
	import { curveNatural } from 'd3-shape';
	import { BarChart, AreaChart } from 'layerchart';
	import { setBreadcrumb } from '$lib/stores/breadcrumb';
	import { toast } from 'svelte-sonner';
	import CloudDownloadIcon from '@lucide/svelte/icons/cloud-download';
	import api from '$lib/api';
	import type { DashBoardResponse, SysInfo, ApiError, TaskStatus, UploadStatus } from '$lib/types';
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
	let uploadedDone = $derived(queue?.done ?? 0);
	let pendingUpload = $derived((queue?.pending ?? 0) + (queue?.waiting ?? 0));
	let uploadPercent = $derived(queue && queue.total > 0 ? Math.round((uploadedDone / queue.total) * 100) : 0);
	let rounds = $derived(
		queue && uploadStatus?.max_per_round
			? (() => {
					const perRound = uploadStatus.max_per_round as number;
					const hours = uploadStatus.interval_seconds ? uploadStatus.interval_seconds / 3600 : 6;
					const r = Math.ceil(queue.pending / perRound);
					return { rounds: r, hours: Math.round(r * hours), days: Math.round((r * hours) / 24) };
				})()
			: null
	);
	let stages = $derived([
		{ name: '已下载', value: queue?.total ?? 0, color: ['#5EE6C0', '#2ED3A8'] },
		{ name: '已上传 115', value: uploadedDone, color: ['#7FB6FF', '#4C8DFF'] },
		{ name: 'strm 就绪', value: Math.max(0, uploadedDone - (queue?.waiting ?? 0)), color: ['#8FA0FF', '#6C7BFF'] },
		{ name: '已清理本地', value: uploadedDone, color: ['#FFC97A', '#FFB84D'] }
	]);
	let days = $derived(dashboardData?.videos_by_day ?? []);
	let maxCnt = $derived(Math.max(1, ...(dashboardData?.videos_by_day ?? []).map((d) => d.cnt)));
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
		const timer = setInterval(loadUploadStatus, 60000);
		return () => clearInterval(timer);
	});
</script>
<div class="min-h-screen bg-gradient-to-br from-[#EAF7F4] via-[#EFF6FB] to-[#E8F1FB] p-4 md:p-6">

	<!-- ========== KPI 行 ========== -->
	<div class="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
		{#each [
			{ label: '订阅总数', value: (dashboardData?.enabled_collections ?? 0) + (dashboardData?.enabled_submissions ?? 0) + (dashboardData?.enabled_favorites ?? 0), sub: `合集 ${dashboardData?.enabled_collections ?? 0} · 投稿 ${dashboardData?.enabled_submissions ?? 0} · 收藏 ${dashboardData?.enabled_favorites ?? 0}`, color: 'from-emerald-400 to-teal-500', icon: VideoIcon },
			{ label: '近 7 日入库', value: totalVideos, sub: '视频', color: 'from-sky-400 to-blue-500', icon: CloudDownloadIcon },
			{ label: '待上传 115', value: pendingUpload, sub: `共 ${queue?.total ?? 0} 集`, color: 'from-violet-400 to-indigo-500', icon: DownloadIcon },
			{ label: '已上传', value: uploadedDone, sub: `${uploadPercent}%`, color: 'from-emerald-400 to-green-500', icon: CircleCheckBigIcon },
			{ label: '等 strm', value: queue?.waiting ?? 0, sub: '未就绪', color: 'from-amber-400 to-orange-500', icon: ClockIcon },
			{ label: '失败', value: queue?.failed ?? 0, sub: '需关注', color: 'from-rose-400 to-red-500', icon: DatabaseIcon }
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

	<!-- ========== 漏斗 + 状态面板 ========== -->
	<div class="mt-4 grid grid-cols-1 gap-4 xl:grid-cols-3">
		<!-- 漏斗 -->
		<div class="rounded-2xl border border-white/70 bg-white/75 p-5 shadow-[0_8px_24px_rgba(31,79,120,0.08)] backdrop-blur xl:col-span-2">
			<div class="mb-4 flex items-center justify-between">
				<h3 class="text-sm font-semibold">内容流转漏斗</h3>
				<span class="text-muted-foreground text-[11px]">已下载 → 已上传 → strm 就绪 → 已清理本地</span>
			</div>
			<div class="flex flex-col items-center gap-2">
				{#each stages as s, i}
					{@const w = 100 - i * 12}
					<div class="relative flex w-full items-center justify-center">
						<div
							class="rounded-lg py-3 text-center text-white shadow-sm transition-all"
							style="width: {w}%; background: linear-gradient(90deg, {s.color[0]}, {s.color[1]});"
						>
							<div class="flex items-center justify-center gap-3">
								<span class="text-xs opacity-90">{s.name}</span>
								<span class="text-lg font-bold tabular-nums">{s.value}</span>
							</div>
						</div>
						<div class="absolute right-0 hidden items-center gap-1 md:flex">
							<span class="rounded-full bg-white/80 px-2 py-0.5 text-[11px] font-medium shadow-sm">
								{stages[0].value > 0 ? Math.round((s.value / stages[0].value) * 100) : 0}%
							</span>
						</div>
					</div>
				{/each}
			</div>
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
				<div class="flex items-center justify-between rounded-xl bg-white/70 px-3 py-2">
					<span class="text-muted-foreground text-xs">下次下载</span>
					<span class="tabular-nums text-xs">{taskStatus?.next_run ? new Date(taskStatus.next_run).toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }) : '-'}</span>
				</div>
				<div class="flex items-center justify-between rounded-xl bg-white/70 px-3 py-2">
					<span class="text-muted-foreground text-xs">上传调度</span>
					<Badge variant={uploadStatus?.available ? 'default' : 'secondary'}>
						{uploadStatus?.available ? (uploadStatus?.enabled ? '已启用' : '已暂停') : '未启用'}
					</Badge>
				</div>
				<div class="flex items-center justify-between rounded-xl bg-white/70 px-3 py-2">
					<span class="text-muted-foreground text-xs">每轮上限</span>
					<span class="tabular-nums text-xs">{uploadStatus?.max_per_round ?? '-'} 集</span>
				</div>
				<div class="flex items-center justify-between rounded-xl bg-white/70 px-3 py-2">
					<span class="text-muted-foreground text-xs">删除本地</span>
					<span class="text-xs">{uploadStatus?.delete_mode === 'delete' ? '就绪后删除' : uploadStatus?.delete_mode === 'recycle' ? '移入回收站' : '保留'}</span>
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
					<span class="text-3xl font-bold tabular-nums">{pendingUpload}</span>
					<span class="text-muted-foreground text-xs">待上传</span>
				</div>
				<div class="h-2.5 w-full overflow-hidden rounded-full bg-slate-200/70">
					<div
						class="h-full rounded-full bg-gradient-to-r from-emerald-400 to-teal-500 transition-all"
						style="width: {uploadPercent}%"
					></div>
				</div>
				<p class="text-muted-foreground mt-1 text-[11px]">已完成 {uploadedDone} / {queue?.total ?? 0}（{uploadPercent}%）</p>
				<div class="mt-3 grid grid-cols-3 gap-2 text-center">
					<div class="rounded-xl bg-white/70 py-2">
						<p class="text-base font-semibold tabular-nums">{round?.uploaded ?? 0}</p>
						<p class="text-muted-foreground text-[10px]">本轮已传</p>
					</div>
					<div class="rounded-xl bg-white/70 py-2">
						<p class="text-base font-semibold tabular-nums">{queue?.waiting ?? 0}</p>
						<p class="text-muted-foreground text-[10px]">等 strm</p>
					</div>
					<div class="rounded-xl bg-white/70 py-2">
						<p class="text-base font-semibold tabular-nums text-rose-500">{queue?.failed ?? 0}</p>
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
					<span class="tabular-nums">{uploadStatus?.interval_seconds ? Math.round(uploadStatus.interval_seconds / 3600) : '-'} 小时</span>
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
				{#if rounds}
					<div class="mt-2 rounded-xl bg-gradient-to-r from-sky-50 to-emerald-50 p-3">
						<p class="text-muted-foreground text-[10px]">预计完成</p>
						<p class="mt-0.5 text-sm font-semibold">
							还需 {rounds.rounds} 轮 · 约 {rounds.days > 0 ? `${rounds.days} 天` : `${rounds.hours} 小时`}
						</p>
					</div>
				{/if}
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

	<!-- ========== 折线 + 存储 ========== -->
	<div class="mt-4 grid grid-cols-1 gap-4 lg:grid-cols-3">
		<div class="rounded-2xl border border-white/70 bg-white/75 p-5 shadow-[0_8px_24px_rgba(31,79,120,0.08)] backdrop-blur lg:col-span-2">
			<div class="mb-2 flex items-center justify-between">
				<h3 class="text-sm font-semibold">近 14 日入库</h3>
				<span class="text-muted-foreground text-[11px]">视频</span>
			</div>
			<div class="flex h-40 items-end gap-1.5">
				{#each days as d}
					<div class="group flex flex-1 flex-col items-center justify-end gap-1">
						<span class="text-muted-foreground text-[9px] tabular-nums opacity-0 group-hover:opacity-100">{d.cnt}</span>
						<div
							class="w-full rounded-t-md bg-gradient-to-t from-sky-300 to-emerald-400 transition-all"
							style="height: {Math.max(4, (d.cnt / maxCnt) * 100)}%"
						></div>
						<span class="text-muted-foreground text-[9px]">{d.day.slice(5)}</span>
					</div>
				{/each}
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
</div>
