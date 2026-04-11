<script setup lang="ts">
import { ref, inject, onMounted, onUnmounted, type Ref } from 'vue'
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

interface SocInfo {
  chip: { type: string }
  rom: { file: string; 'version-bsp'?: string }
  script: { file: string; bitw?: number; 'use-luac'?: boolean }
  download: { bl_addr?: string; script_addr?: string; force_br?: string }
  user?: { log_br?: string }
}

interface FlashEvent {
  stage: string
  percent: number
  message: string
  done: boolean
  error: boolean
}

interface TestResult {
  passed: boolean
  lines: string[]
  message: string
}

// ─── Shared state from App.vue ────────────────────────────────────────────────

const shared = inject<{
  selectedPort: Ref<string>
  logBaud: Ref<number>
  switchToLog: () => void
  startLogCapture: (port: string, baud: number) => void
}>('sharedState')!

// ─── Local state ──────────────────────────────────────────────────────────────

const socPath     = ref('')
const socInfo     = ref<SocInfo | null>(null)
const socError    = ref<string | null>(null)

const scriptFolder = ref('')

const ports        = ref<PortInfo[]>([])
const selectedPort = shared.selectedPort
const isLoadingPorts = ref(false)
const portError    = ref<string | null>(null)

const isFlashing   = ref(false)
const flashDone    = ref(false)
const flashError   = ref(false)

const flashStage   = ref('')
const flashPercent = ref(0)
const flashLog     = ref<string[]>([])

const autoTest     = ref(false)
const testResult   = ref<TestResult | null>(null)
const testRunning  = ref(false)

let unlistenFlash: UnlistenFn | null = null

// ─── Serial port helpers ──────────────────────────────────────────────────────

async function refreshPorts() {
  isLoadingPorts.value = true
  portError.value = null
  try {
    const result = await invoke<PortInfo[]>('get_serial_ports')
    ports.value = result
    if (!result.find((p: PortInfo) => p.port_name === selectedPort.value)) {
      selectedPort.value = result[0]?.port_name ?? ''
    }
    if (result.length === 0) portError.value = 'No serial ports found.'
  } catch (e) {
    portError.value = String(e)
  } finally {
    isLoadingPorts.value = false
  }
}

function portLabel(p: PortInfo) {
  const parts = [p.port_name]
  if (p.product) parts.push(p.product)
  else if (p.manufacturer) parts.push(p.manufacturer)
  if (p.vid && p.pid) parts.push(`[${p.vid}:${p.pid}]`)
  return parts.join(' — ')
}

// ─── .soc selection & info ───────────────────────────────────────────────────

async function browseSoc() {
  const path = await invoke<string | null>('open_file_dialog', {
    title: '选择固件 (.soc)',
    filterName: 'LuatOS 固件',
    extensions: ['soc'],
  })
  if (path) { socPath.value = path; await loadSocInfo() }
}

async function loadSocInfo() {
  socError.value = null
  socInfo.value = null
  try {
    socInfo.value = await invoke<SocInfo>('get_soc_info', { socPath: socPath.value })
    // Pre-fill log baud rate from firmware info
    const logBr = socInfo.value?.user?.log_br
    if (logBr) shared.logBaud.value = parseInt(logBr, 10) || 2000000
  } catch (e) {
    socError.value = String(e)
  }
}

// ─── Script folder ────────────────────────────────────────────────────────────

async function browseScript() {
  const path = await invoke<string | null>('open_folder_dialog', {
    title: '选择 Lua 脚本目录',
  })
  if (path) scriptFolder.value = path
}

// ─── Flash ────────────────────────────────────────────────────────────────────

async function startFlash() {
  if (!socPath.value) { socError.value = '请先选择 .soc 固件文件'; return }
  if (!selectedPort.value) { portError.value = '请先选择串口'; return }

  isFlashing.value = true
  flashDone.value  = false
  flashError.value = false
  flashStage.value = 'Preparing'
  flashPercent.value = 0
  flashLog.value   = []
  testResult.value = null

  // Register progress listener
  unlistenFlash = await listen<FlashEvent>('flash:progress', (evt: Event<FlashEvent>) => {
    const { stage, percent, message, done, error } = evt.payload
    if (message) flashLog.value.push(message)
    if (percent >= 0) flashPercent.value = Math.round(percent)
    if (stage) flashStage.value = stage
    if (done) {
      isFlashing.value = false
      flashDone.value  = true
      flashError.value = error
      if (!error && autoTest.value) afterFlashTest()
    }
  })

  try {
    await invoke('flash_device', {
      socPath: socPath.value,
      scriptFolder: scriptFolder.value || null,
      port: selectedPort.value,
      baudRate: null,
    })
  } catch (e) {
    isFlashing.value = false
    flashDone.value  = true
    flashError.value = true
    flashLog.value.push(`ERROR: ${e}`)
  }
}

async function cancelFlash() {
  await invoke('cancel_flash')
  isFlashing.value = false
  flashLog.value.push('[取消] 刷机已取消')
}

// ─── Auto test (flash + boot verify) ─────────────────────────────────────────

async function afterFlashTest() {
  testRunning.value = true
  testResult.value  = null
  const logBr = socInfo.value?.user?.log_br
    ? parseInt(socInfo.value.user.log_br, 10) : 2000000

  // Switch to log view so user can watch the boot output
  shared.logBaud.value = logBr
  shared.switchToLog()
  shared.startLogCapture(selectedPort.value, logBr)

  try {
    const result = await invoke<TestResult>('run_flash_test', {
      socPath: socPath.value,
      port: selectedPort.value,
      timeoutSecs: 20,
    })
    testResult.value = result
  } catch (e) {
    testResult.value = { passed: false, lines: [], message: String(e) }
  } finally {
    testRunning.value = false
  }
}

async function runTestNow() {
  if (!socPath.value || !selectedPort.value) return
  testRunning.value = true
  testResult.value = null
  flashLog.value = []
  try {
    const result = await invoke<TestResult>('run_flash_test', {
      socPath: socPath.value,
      port: selectedPort.value,
      timeoutSecs: 20,
    })
    testResult.value = result
    flashLog.value = result.lines.slice(-30)
  } catch (e) {
    testResult.value = { passed: false, lines: [], message: String(e) }
  } finally {
    testRunning.value = false
  }
}

// ─── Lifecycle ────────────────────────────────────────────────────────────────

onMounted(refreshPorts)
onUnmounted(() => { unlistenFlash?.() })
</script>

<template>
  <div class="space-y-5">
    <h1 class="text-xl font-bold text-gray-100">下载 / 刷机</h1>

    <!-- ── Firmware (.soc) ──────────────────────────────────────────────── -->
    <div class="bg-gray-900 border border-gray-800 rounded-lg p-4 space-y-3">
      <h2 class="text-xs font-semibold text-cyan-400 uppercase tracking-wider">固件 (.soc)</h2>
      <div class="flex gap-2">
        <input
          v-model="socPath"
          type="text"
          readonly
          placeholder="未选择固件..."
          @click="browseSoc"
          class="flex-1 bg-gray-800 border border-gray-700 rounded px-3 py-2 text-sm text-gray-300 placeholder-gray-600 cursor-pointer focus:outline-none focus:border-cyan-600"
        />
        <button @click="browseSoc" class="px-3 py-2 bg-gray-700 hover:bg-gray-600 rounded text-sm text-gray-200 transition-colors">浏览</button>
      </div>
      <p v-if="socError" class="text-xs text-red-400">{{ socError }}</p>
      <div v-if="socInfo" class="flex flex-wrap gap-3 text-xs">
        <span class="bg-gray-800 rounded px-2 py-1 text-cyan-300">
          芯片: {{ socInfo.chip.type }}
        </span>
        <span v-if="socInfo.rom['version-bsp']" class="bg-gray-800 rounded px-2 py-1 text-green-300">
          版本: {{ socInfo.rom['version-bsp'] }}
        </span>
        <span v-if="socInfo.download.force_br" class="bg-gray-800 rounded px-2 py-1 text-yellow-300">
          刷机波特率: {{ socInfo.download.force_br }}
        </span>
        <span v-if="socInfo.user?.log_br" class="bg-gray-800 rounded px-2 py-1 text-purple-300">
          日志波特率: {{ socInfo.user.log_br }}
        </span>
        <span class="bg-gray-800 rounded px-2 py-1 text-gray-400">
          ROM: {{ socInfo.rom.file }}
        </span>
        <span v-if="socInfo.script.bitw" class="bg-gray-800 rounded px-2 py-1 text-gray-400">
          Lua {{ socInfo.script.bitw }}bit
        </span>
      </div>
    </div>

    <!-- ── Script folder (optional) ────────────────────────────────────── -->
    <div class="bg-gray-900 border border-gray-800 rounded-lg p-4 space-y-3">
      <h2 class="text-xs font-semibold text-cyan-400 uppercase tracking-wider">
        Lua 脚本目录 <span class="text-gray-600 normal-case font-normal ml-1">(可选 — 留空则只刷固件)</span>
      </h2>
      <div class="flex gap-2">
        <input
          v-model="scriptFolder"
          type="text"
          readonly
          placeholder="拖拽 Lua 目录或点击选择..."
          @click="browseScript"
          class="flex-1 bg-gray-800 border border-gray-700 rounded px-3 py-2 text-sm text-gray-300 placeholder-gray-600 cursor-pointer focus:outline-none focus:border-cyan-600"
        />
        <button @click="browseScript" class="px-3 py-2 bg-gray-700 hover:bg-gray-600 rounded text-sm text-gray-200 transition-colors">浏览</button>
        <button v-if="scriptFolder" @click="scriptFolder = ''" class="px-2 py-2 bg-gray-700 hover:bg-red-800 rounded text-sm text-gray-400 transition-colors">✕</button>
      </div>
    </div>

    <!-- ── Device port ──────────────────────────────────────────────────── -->
    <div class="bg-gray-900 border border-gray-800 rounded-lg p-4 space-y-2">
      <h2 class="text-xs font-semibold text-cyan-400 uppercase tracking-wider">设备串口</h2>
      <div class="flex gap-2">
        <select
          v-model="selectedPort"
          :disabled="isLoadingPorts || isFlashing"
          class="bg-gray-800 border border-gray-700 rounded px-3 py-2 text-sm text-gray-300 focus:outline-none min-w-[240px]"
        >
          <option value="">— 请选择串口 —</option>
          <option v-for="p in ports" :key="p.port_name" :value="p.port_name">
            {{ portLabel(p) }}
          </option>
        </select>
        <button @click="refreshPorts" :disabled="isLoadingPorts || isFlashing"
          class="px-3 py-2 bg-gray-700 hover:bg-gray-600 disabled:opacity-40 rounded text-sm text-gray-200 transition-colors">
          {{ isLoadingPorts ? '...' : '刷新' }}
        </button>
      </div>
      <p v-if="portError" class="text-xs text-yellow-400">{{ portError }}</p>
    </div>

    <!-- ── Action buttons ───────────────────────────────────────────────── -->
    <div class="flex gap-3 items-center">
      <button
        v-if="!isFlashing"
        @click="startFlash"
        :disabled="!socPath || !selectedPort"
        class="flex-1 py-3 bg-cyan-600 hover:bg-cyan-500 active:bg-cyan-700 disabled:opacity-40 disabled:cursor-not-allowed rounded-lg text-base font-bold text-white transition-colors"
      >
        ⚡ 开始刷机
      </button>
      <button
        v-else
        @click="cancelFlash"
        class="flex-1 py-3 bg-red-700 hover:bg-red-600 rounded-lg text-base font-bold text-white transition-colors"
      >
        ✕ 取消刷机
      </button>

      <button
        @click="runTestNow"
        :disabled="!socPath || !selectedPort || isFlashing || testRunning"
        class="px-4 py-3 bg-emerald-700 hover:bg-emerald-600 disabled:opacity-40 disabled:cursor-not-allowed rounded-lg text-sm font-bold text-white transition-colors"
        title="刷机 + 验证启动 (闭环测试)"
      >
        {{ testRunning ? '⏳ 测试中...' : '🔬 闭环测试' }}
      </button>

      <label class="flex items-center gap-2 text-sm text-gray-400 cursor-pointer select-none">
        <input type="checkbox" v-model="autoTest" class="accent-cyan-500" />
        刷机后自动测试
      </label>
    </div>

    <!-- ── Progress ─────────────────────────────────────────────────────── -->
    <div class="bg-gray-900 border border-gray-800 rounded-lg p-4 space-y-3">
      <div class="flex items-center justify-between">
        <h2 class="text-xs font-semibold text-cyan-400 uppercase tracking-wider">进度</h2>
        <span class="text-xs font-mono" :class="flashError ? 'text-red-400' : flashDone ? 'text-green-400' : 'text-gray-500'">
          {{ flashDone ? (flashError ? '❌ 失败' : '✅ 完成') : (isFlashing ? flashStage : '等待开始') }}
        </span>
      </div>

      <!-- Progress bar -->
      <div class="w-full bg-gray-800 rounded-full h-2.5">
        <div
          class="h-2.5 rounded-full transition-all duration-300"
          :class="flashError ? 'bg-red-500' : flashDone ? 'bg-green-500' : 'bg-cyan-500'"
          :style="{ width: flashPercent + '%' }"
        />
      </div>
      <p class="text-xs text-gray-500 font-mono">
        {{ isFlashing ? `${flashStage} — ${flashPercent}%` : flashDone ? (flashError ? '刷机失败' : '刷机成功') : '等待开始...' }}
      </p>

      <!-- Flash log (last 15 lines) -->
      <div v-if="flashLog.length > 0"
        class="bg-gray-950 rounded p-3 font-mono text-xs leading-5 max-h-48 overflow-y-auto space-y-0.5">
        <div v-for="(line, i) in flashLog.slice(-20)" :key="i"
          :class="line.includes('ERROR') || line.includes('Failed') ? 'text-red-400' :
                  line.includes('WARN') || line.includes('Warning') ? 'text-yellow-400' :
                  line.includes('OK') || line.includes('Finished') || line.includes('complete') ? 'text-green-400' :
                  'text-gray-400'">
          {{ line }}
        </div>
      </div>
    </div>

    <!-- ── Test result ───────────────────────────────────────────────────── -->
    <div v-if="testResult"
      class="rounded-lg p-4 border"
      :class="testResult.passed ? 'border-green-700 bg-green-900/20' : 'border-red-700 bg-red-900/20'">
      <div class="flex items-center gap-2 font-bold text-sm"
        :class="testResult.passed ? 'text-green-400' : 'text-red-400'">
        {{ testResult.passed ? '✅ TEST PASS' : '❌ TEST FAIL' }}
        <span class="font-normal text-gray-400 text-xs ml-2">{{ testResult.message }}</span>
      </div>
      <div v-if="testResult.lines.length > 0"
        class="mt-2 bg-gray-950 rounded p-2 font-mono text-xs leading-5 max-h-32 overflow-y-auto">
        <div v-for="(l, i) in testResult.lines.slice(0, 20)" :key="i" class="text-gray-400">{{ l }}</div>
      </div>
    </div>
  </div>
</template>

