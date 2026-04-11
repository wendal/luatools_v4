<script setup lang="ts">
import { ref, inject, onMounted, onUnmounted, nextTick, watch, type Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn, type Event } from '@tauri-apps/api/event'

// ─── Types ────────────────────────────────────────────────────────────────────

interface PortInfo {
  port_name: string
  vid: string | null
  pid: string | null
  manufacturer: string | null
  product: string | null
  serial_number: string | null
}

interface LogLine {
  text: string
  timestamp_ms: number
}

interface LogStatus {
  connected: boolean
  message: string
}

// ─── Shared state ─────────────────────────────────────────────────────────────

const shared = inject<{
  selectedPort: Ref<string>
  logBaud: Ref<number>
  pendingLogStart: Ref<{ port: string; baud: number } | null>
}>('sharedState')!

// ─── Local state ──────────────────────────────────────────────────────────────

const ports        = ref<PortInfo[]>([])
const selectedPort = shared.selectedPort
const baudRate     = shared.logBaud
const isLoading    = ref(false)

const isRunning    = ref(false)
const isConnected  = ref(false)
const statusMsg    = ref('')

const lines        = ref<{ text: string; ts: string }[]>([])
const filter       = ref('')
const autoScroll   = ref(true)
const maxLines     = 2000

const logContainer = ref<HTMLElement | null>(null)

let unlistenLine: UnlistenFn | null = null
let unlistenStatus: UnlistenFn | null = null

const startTime = ref(0)

// ─── Port helpers ─────────────────────────────────────────────────────────────

async function refreshPorts() {
  isLoading.value = true
  try {
    const result = await invoke<PortInfo[]>('get_serial_ports')
    ports.value = result
    if (!result.find((p: PortInfo) => p.port_name === selectedPort.value)) {
      selectedPort.value = result[0]?.port_name ?? ''
    }
  } finally {
    isLoading.value = false
  }
}

function portLabel(p: PortInfo) {
  const parts = [p.port_name]
  if (p.product) parts.push(p.product)
  else if (p.manufacturer) parts.push(p.manufacturer)
  if (p.vid && p.pid) parts.push(`[${p.vid}:${p.pid}]`)
  return parts.join(' — ')
}

// ─── Log control ─────────────────────────────────────────────────────────────

async function startLog() {
  if (!selectedPort.value) return
  isRunning.value = true
  startTime.value = Date.now()
  try {
    await invoke('start_serial_log', {
      port: selectedPort.value,
      baudRate: baudRate.value,
    })
  } catch (e) {
    isRunning.value = false
    statusMsg.value = String(e)
  }
}

async function stopLog() {
  await invoke('stop_serial_log')
  isRunning.value = false
  isConnected.value = false
}

function clearLog() {
  lines.value = []
}

function formatTs(ms: number): string {
  const total = Math.floor(ms / 1000)
  const s = total % 60
  const m = Math.floor(total / 60) % 60
  const h = Math.floor(total / 3600)
  const frac = String(ms % 1000).padStart(3, '0')
  return `${String(h).padStart(2,'0')}:${String(m).padStart(2,'0')}:${String(s).padStart(2,'0')}.${frac}`
}

function lineClass(text: string): string {
  if (text.startsWith('E/') || text.includes('[ERROR]')) return 'text-red-400'
  if (text.startsWith('W/') || text.includes('[WARN]'))  return 'text-yellow-400'
  if (text.startsWith('I/') || text.includes('[INFO]'))  return 'text-cyan-300'
  if (text.startsWith('D/'))                             return 'text-gray-500'
  return 'text-gray-300'
}

async function scrollToBottom() {
  if (!autoScroll.value || !logContainer.value) return
  await nextTick()
  logContainer.value.scrollTop = logContainer.value.scrollHeight
}

// ─── Filtered lines ───────────────────────────────────────────────────────────

const filteredLines = () => {
  if (!filter.value) return lines.value
  const f = filter.value.toLowerCase()
  return lines.value.filter(l => l.text.toLowerCase().includes(f))
}

// ─── Event listeners ──────────────────────────────────────────────────────────

async function setupListeners() {
  unlistenLine = await listen<LogLine>('log:line', (evt: Event<LogLine>) => {
    const { text, timestamp_ms } = evt.payload
    lines.value.push({ text, ts: formatTs(timestamp_ms) })
    if (lines.value.length > maxLines) {
      lines.value.splice(0, lines.value.length - maxLines)
    }
    scrollToBottom()
  })

  unlistenStatus = await listen<LogStatus>('log:status', (evt: Event<LogStatus>) => {
    const { connected, message } = evt.payload
    isConnected.value = connected
    statusMsg.value = message
    if (!connected) isRunning.value = false
  })
}

// ─── Watch for pending log start from FlashView ───────────────────────────────

watch(() => shared.pendingLogStart.value, async (pending) => {
  if (pending) {
    shared.pendingLogStart.value = null
    selectedPort.value = pending.port
    baudRate.value = pending.baud
    await startLog()
  }
})

// ─── Lifecycle ────────────────────────────────────────────────────────────────

onMounted(async () => {
  await refreshPorts()
  await setupListeners()
})

onUnmounted(() => {
  unlistenLine?.()
  unlistenStatus?.()
  invoke('stop_serial_log').catch(() => {})
})

// ─── Export for use from App.vue ──────────────────────────────────────────────

defineExpose({ startLog, stopLog })
</script>

<template>
  <div class="flex flex-col h-full space-y-3">

    <!-- ── Header bar ──────────────────────────────────────────────────── -->
    <div class="flex items-center justify-between">
      <h1 class="text-xl font-bold text-gray-100">日志查看器</h1>
      <div class="flex items-center gap-1.5">
        <span class="w-2 h-2 rounded-full" :class="isConnected ? 'bg-green-400 animate-pulse' : 'bg-gray-600'" />
        <span class="text-xs text-gray-500">{{ statusMsg || (isConnected ? '已连接' : '未连接') }}</span>
      </div>
    </div>

    <!-- ── Controls ────────────────────────────────────────────────────── -->
    <div class="bg-gray-900 border border-gray-800 rounded-lg p-3 flex flex-wrap gap-2 items-center">
      <!-- Port selector -->
      <select
        v-model="selectedPort"
        :disabled="isRunning || isLoading"
        class="bg-gray-800 border border-gray-700 rounded px-3 py-1.5 text-sm text-gray-300 focus:outline-none"
      >
        <option value="">— 串口 —</option>
        <option v-for="p in ports" :key="p.port_name" :value="p.port_name">
          {{ portLabel(p) }}
        </option>
      </select>

      <!-- Baud rate -->
      <select
        v-model.number="baudRate"
        :disabled="isRunning"
        class="bg-gray-800 border border-gray-700 rounded px-3 py-1.5 text-sm text-gray-300 focus:outline-none w-32"
      >
        <option :value="2000000">2000000</option>
        <option :value="921600">921600</option>
        <option :value="115200">115200</option>
        <option :value="9600">9600</option>
      </select>

      <!-- Refresh ports -->
      <button @click="refreshPorts" :disabled="isRunning"
        class="px-2 py-1.5 bg-gray-700 hover:bg-gray-600 disabled:opacity-40 rounded text-xs text-gray-300 transition-colors">
        刷新
      </button>

      <!-- Start / Stop -->
      <button v-if="!isRunning" @click="startLog" :disabled="!selectedPort"
        class="px-4 py-1.5 bg-cyan-600 hover:bg-cyan-500 disabled:opacity-40 rounded text-sm font-semibold text-white transition-colors">
        ▶ 开始
      </button>
      <button v-else @click="stopLog"
        class="px-4 py-1.5 bg-orange-600 hover:bg-orange-500 rounded text-sm font-semibold text-white transition-colors">
        ■ 停止
      </button>

      <div class="ml-auto flex items-center gap-2">
        <label class="flex items-center gap-1 text-xs text-gray-500 cursor-pointer">
          <input type="checkbox" v-model="autoScroll" class="accent-cyan-500" />
          自动滚动
        </label>
        <span class="text-xs text-gray-600">{{ lines.length }} 行</span>
        <button @click="clearLog" class="px-2 py-1.5 bg-gray-800 hover:bg-gray-700 rounded text-xs text-gray-400 transition-colors">
          清空
        </button>
      </div>
    </div>

    <!-- ── Filter ───────────────────────────────────────────────────────── -->
    <input
      v-model="filter"
      type="text"
      placeholder="过滤关键字..."
      class="bg-gray-900 border border-gray-800 rounded px-3 py-2 text-sm text-gray-300 placeholder-gray-600 focus:outline-none focus:border-cyan-600"
    />

    <!-- ── Log terminal ──────────────────────────────────────────────────── -->
    <div
      ref="logContainer"
      class="flex-1 bg-gray-950 border border-gray-800 rounded-lg p-3 overflow-y-auto font-mono text-xs leading-5 min-h-0"
      style="min-height: 200px"
    >
      <div
        v-for="(entry, i) in filteredLines()"
        :key="i"
        :class="lineClass(entry.text)"
        class="flex gap-2 hover:bg-gray-900/50"
      >
        <span class="text-gray-700 select-none shrink-0">{{ entry.ts }}</span>
        <span class="break-all">{{ entry.text }}</span>
      </div>
      <div v-if="lines.length === 0" class="text-gray-700 italic">
        {{ isRunning ? '等待设备输出...' : '点击"开始"连接串口查看日志' }}
      </div>
    </div>
  </div>
</template>

