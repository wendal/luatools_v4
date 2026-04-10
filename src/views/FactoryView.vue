<script setup lang="ts">
import { ref } from 'vue'

interface DeviceSlot {
  id: number
  port: string | null
  status: 'idle' | 'flashing' | 'success' | 'error'
  progress: number
}

const slots = ref<DeviceSlot[]>(
  Array.from({ length: 8 }, (_, i) => ({
    id: i + 1,
    port: null,
    status: 'idle',
    progress: 0,
  }))
)

const statusColor: Record<DeviceSlot['status'], string> = {
  idle:     'bg-gray-800 border-gray-700 text-gray-500',
  flashing: 'bg-blue-900  border-blue-600  text-blue-200',
  success:  'bg-green-900 border-green-500 text-green-200',
  error:    'bg-red-900   border-red-500   text-red-200',
}

const statusLabel: Record<DeviceSlot['status'], string> = {
  idle:     'IDLE',
  flashing: 'FLASHING…',
  success:  '✓ OK',
  error:    '✗ FAIL',
}
</script>

<template>
  <div class="space-y-6">
    <h1 class="text-xl font-bold text-gray-100">Factory Mode</h1>
    <p class="text-sm text-gray-500">
      Plug devices into USB hub slots. Each connected device will be flashed automatically.
    </p>

    <div class="grid grid-cols-2 sm:grid-cols-4 gap-4">
      <div
        v-for="slot in slots"
        :key="slot.id"
        :class="['border rounded-xl p-4 flex flex-col items-center gap-2 transition-colors', statusColor[slot.status]]"
      >
        <span class="text-3xl font-bold">{{ slot.id }}</span>
        <span class="text-xs font-semibold tracking-widest">{{ statusLabel[slot.status] }}</span>
        <span class="text-xs opacity-60">{{ slot.port ?? '—' }}</span>
        <div class="w-full bg-gray-700 rounded-full h-1 mt-1">
          <div
            class="bg-current h-1 rounded-full transition-all duration-300"
            :style="{ width: slot.progress + '%' }"
          />
        </div>
      </div>
    </div>
  </div>
</template>
