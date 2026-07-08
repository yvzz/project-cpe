/*
 * @Author: 1orz cloudorzi@gmail.com
 * @Date: 2025-12-09 17:34:01
 * @LastEditors: 1orz cloudorzi@gmail.com
 * @LastEditTime: 2025-12-13 12:44:50
 * @FilePath: /udx710-backend/frontend/src/pages/Configuration.tsx
 * @Description: 
 * 
 * Copyright (c) 2025 by 1orz, All Rights Reserved. 
 */
import { useEffect, useState, type ChangeEvent, type MouseEvent } from 'react'
import {
  Box,
  Typography,
  Accordion,
  AccordionSummary,
  AccordionDetails,
  Switch,
  FormControlLabel,
  Radio,
  RadioGroup,
  FormControl,
  FormLabel,
  Button,
  Divider,
  Alert,
  CircularProgress,
  Chip,
  Snackbar,
  Card,
  CardContent,
  CardHeader,
  LinearProgress,
  TextField,
} from '@mui/material'
import Grid from '@mui/material/Grid'
import {
  ExpandMore,
  Wifi,
  Usb,
  CheckCircle,
  Error as ErrorIcon,
  FlashOn,
  HealthAndSafety,
  FlightTakeoff,
  Webhook,
  PlayArrow,
  Chat,
  Email,
  Smartphone,
  Block,
} from '@mui/icons-material'
import { api } from '../api'
import ErrorSnackbar from '../components/ErrorSnackbar'
import type { UsbModeResponse, AirplaneModeResponse, NotificationChannel, ChannelType, DingtalkConfig, FeishuConfig, WecomConfig, EmailConfig, BarkConfig } from '../api/types'
import { DEFAULT_NOTIFICATION_CHANNEL } from '../api/types'

// ========== 通知渠道辅助组件 ==========

const CHANNEL_OPTIONS: { value: ChannelType; label: string; icon: React.ReactNode; desc: string }[] = [
  {
    value: 'none',
    label: '不使用',
    icon: <Block />,
    desc: '关闭推送通知'
  },
  {
    value: 'dingtalk',
    label: '钉钉群机器人',
    icon: <Chat />,
    desc: '通过钉钉群机器人推送消息'
  },
  {
    value: 'feishu',
    label: '飞书群机器人',
    icon: <Chat />,
    desc: '通过飞书群机器人推送消息'
  },
  {
    value: 'wecom',
    label: '企业微信机器人',
    icon: <Chat />,
    desc: '通过企业微信群机器人推送消息'
  },
  {
    value: 'email',
    label: '邮件转发',
    icon: <Email />,
    desc: '通过SMTP邮件发送通知'
  },
  {
    value: 'bark',
    label: 'Bark (iOS推送)',
    icon: <Smartphone />,
    desc: '通过Bark推送到iOS设备'
  },
]

function DingtalkForm({ config, onChange }: { config: DingtalkConfig, onChange: (c: DingtalkConfig) => void }) {
  return (
    <Box>
      <Typography variant="body2" color="text.secondary" mb={2}>
        在钉钉群中添加「自定义机器人」，复制 Webhook 地址和加签密钥到下方。
      </Typography>
      <TextField fullWidth label="Webhook URL" value={config.url}
        onChange={e => onChange({ ...config, url: e.target.value })}
        placeholder="https://oapi.dingtalk.com/robot/send?access_token=xxx"
        sx={{ mb: 2 }} />
      <TextField fullWidth label="加签密钥 (Secret)" value={config.secret}
        onChange={e => onChange({ ...config, secret: e.target.value })}
        type="password" placeholder="SEC..." helperText="钉钉机器人安全设置中的加签密钥"
        sx={{ mb: 2 }} />
      <TextField fullWidth label="自定义模板 (可选)" value={config.template}
        onChange={e => onChange({ ...config, template: e.target.value })}
        multiline rows={4} placeholder='默认：{"msgtype":"text","text":{"content":"📱 短信\n发送方: {{phone_number}}\n内容: {{content}}"}}'
        InputProps={{ sx: { fontFamily: 'monospace', fontSize: '0.85rem' } }} />
    </Box>
  )
}

function FeishuForm({ config, onChange }: { config: FeishuConfig, onChange: (c: FeishuConfig) => void }) {
  return (
    <Box>
      <Typography variant="body2" color="text.secondary" mb={2}>
        在飞书群中添加「自定义机器人」，复制 Webhook 地址和签名密钥到下方。
      </Typography>
      <TextField fullWidth label="Webhook URL" value={config.url}
        onChange={e => onChange({ ...config, url: e.target.value })}
        placeholder="https://open.feishu.cn/open-apis/bot/v2/hook/xxx"
        sx={{ mb: 2 }} />
      <TextField fullWidth label="签名密钥 (可选)" value={config.secret}
        onChange={e => onChange({ ...config, secret: e.target.value })}
        type="password" placeholder="留空则不签名"
        helperText="飞书机器人安全设置中的签名密钥（可选）"
        sx={{ mb: 2 }} />
      <TextField fullWidth label="自定义模板 (可选)" value={config.template}
        onChange={e => onChange({ ...config, template: e.target.value })}
        multiline rows={4} placeholder='默认：{"msg_type":"text","content":{"text":"..."}}'
        InputProps={{ sx: { fontFamily: 'monospace', fontSize: '0.85rem' } }} />
    </Box>
  )
}

function WecomForm({ config, onChange }: { config: WecomConfig, onChange: (c: WecomConfig) => void }) {
  return (
    <Box>
      <Typography variant="body2" color="text.secondary" mb={2}>
        在企业微信群中添加「群机器人」，复制 Webhook 地址和密钥到下方。
      </Typography>
      <TextField fullWidth label="Webhook URL" value={config.url}
        onChange={e => onChange({ ...config, url: e.target.value })}
        placeholder="https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=xxx"
        sx={{ mb: 2 }} />
      <TextField fullWidth label="签名密钥 (可选)" value={config.secret}
        onChange={e => onChange({ ...config, secret: e.target.value })}
        type="password" placeholder="留空则不签名"
        sx={{ mb: 2 }} />
      <TextField fullWidth label="自定义模板 (可选)" value={config.template}
        onChange={e => onChange({ ...config, template: e.target.value })}
        multiline rows={4}
        InputProps={{ sx: { fontFamily: 'monospace', fontSize: '0.85rem' } }} />
    </Box>
  )
}

function EmailForm({ config, onChange }: { config: EmailConfig, onChange: (c: EmailConfig) => void }) {
  return (
    <Box>
      <Typography variant="body2" color="text.secondary" mb={2}>
        配置 SMTP 服务器发送邮件通知。支持 QQ邮箱、163邮箱、Gmail 等主流邮箱。
      </Typography>
      <Grid container spacing={2}>
        <Grid size={{ xs: 12, md: 6 }}>
          <TextField fullWidth label="SMTP 服务器" value={config.smtp_host}
            onChange={e => onChange({ ...config, smtp_host: e.target.value })}
            placeholder="smtp.qq.com" />
        </Grid>
        <Grid size={{ xs: 12, md: 6 }}>
          <TextField fullWidth label="SMTP 端口" value={config.smtp_port}
            onChange={e => onChange({ ...config, smtp_port: Number(e.target.value) })}
            type="number" placeholder="465" />
        </Grid>
      </Grid>
      <FormControlLabel
        control={<Switch checked={config.use_tls} onChange={e => onChange({ ...config, use_tls: e.target.checked })} />}
        label="使用 SSL/TLS（推荐 465 端口）"
        sx={{ my: 1 }}
      />
      {!config.use_tls && (
        <Alert severity="info" sx={{ mb: 2 }}>使用 STARTTLS（587 端口），需服务器支持 TLS 连接</Alert>
      )}
      <Grid container spacing={2}>
        <Grid size={{ xs: 12, md: 6 }}>
          <TextField fullWidth label="发件人邮箱" value={config.username}
            onChange={e => onChange({ ...config, username: e.target.value })}
            placeholder="your@email.com" />
        </Grid>
        <Grid size={{ xs: 12, md: 6 }}>
          <TextField fullWidth label="发件人显示名" value={config.from_name}
            onChange={e => onChange({ ...config, from_name: e.target.value })}
            placeholder="CPE通知" />
        </Grid>
      </Grid>
      <TextField fullWidth label="授权码/密码" value={config.password}
        onChange={e => onChange({ ...config, password: e.target.value })}
        type="password" placeholder="SMTP授权码（非登录密码）" sx={{ mt: 2 }}
        helperText="QQ邮箱：设置 → 账户 → POP3/IMAP/SMTP/Exchange/CardDAV/CalDAV服务 → 生成授权码" />
      <TextField fullWidth label="收件人邮箱" value={config.to_addresses}
        onChange={e => onChange({ ...config, to_addresses: e.target.value })}
        placeholder="recipient@example.com（多个用逗号分隔）" sx={{ mt: 2 }}
        helperText="支持多个收件人，用英文逗号分隔" />
      <TextField fullWidth label="邮件主题前缀" value={config.subject_prefix}
        onChange={e => onChange({ ...config, subject_prefix: e.target.value })}
        placeholder="[CPE通知]" sx={{ mt: 2 }} />
    </Box>
  )
}

function BarkForm({ config, onChange }: { config: BarkConfig, onChange: (c: BarkConfig) => void }) {
  return (
    <Box>
      <Typography variant="body2" color="text.secondary" mb={2}>
        Bark 是一款 iOS 推送服务，需要先在 App Store 安装 Bark 并获取设备 Key。
      </Typography>
      <Alert severity="info" sx={{ mb: 2 }}>
        <Typography variant="body2">
          <strong>设置步骤：</strong><br/>
          1. 在 iOS 设备上打开 Bark，复制显示的「Server URL」和「Device Key」<br/>
          2. 下方填入对应内容即可使用
        </Typography>
      </Alert>
      <TextField fullWidth label="Bark 服务器地址" value={config.server_url}
        onChange={e => onChange({ ...config, server_url: e.target.value })}
        placeholder="https://api.day.app" sx={{ mb: 2 }} />
      <TextField fullWidth label="设备 Key / Token" value={config.device_key}
        onChange={e => onChange({ ...config, device_key: e.target.value })}
        placeholder="Device Key" sx={{ mb: 2 }} />
      <Grid container spacing={2}>
        <Grid size={{ xs: 12, md: 6 }}>
          <TextField fullWidth label="推送铃声 (可选)" value={config.sound}
            onChange={e => onChange({ ...config, sound: e.target.value })}
            placeholder="留空使用默认铃声" />
        </Grid>
        <Grid size={{ xs: 12, md: 6 }}>
          <TextField fullWidth label="分组名称 (可选)" value={config.group}
            onChange={e => onChange({ ...config, group: e.target.value })}
            placeholder="CPE" />
        </Grid>
      </Grid>
      <TextField fullWidth label="图标 URL (可选)" value={config.icon}
        onChange={e => onChange({ ...config, icon: e.target.value })}
        placeholder="https://example.com/icon.png" sx={{ mt: 2 }} />
    </Box>
  )
}

interface HealthStatus {
  status: string
  timestamp?: string
}

export default function ConfigurationPage() {
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [success, setSuccess] = useState<string | null>(null)
  const [expanded, setExpanded] = useState<string | false>('dataConnection')
  
  const [dataStatus, setDataStatus] = useState(false)
  const [usbMode, setUsbMode] = useState<UsbModeResponse | null>(null)
  const [selectedUsbMode, setSelectedUsbMode] = useState<number>(1)
  const [usbModePermanent, setUsbModePermanent] = useState<boolean>(false)
  const [useHotSwitch, setUseHotSwitch] = useState<boolean>(false)
  const [rebooting, setRebooting] = useState(false)
  const [hotSwitching, setHotSwitching] = useState(false)
  
  // 飞行模式状态
  const [airplaneMode, setAirplaneMode] = useState<AirplaneModeResponse | null>(null)
  const [airplaneSwitching, setAirplaneSwitching] = useState(false)
  
  // 健康检查状态
  const [healthStatus, setHealthStatus] = useState<HealthStatus | null>(null)
  const [healthLoading, setHealthLoading] = useState(false)

  // 通知渠道配置状态
  const [notificationChannel, setNotificationChannel] = useState<NotificationChannel>(DEFAULT_NOTIFICATION_CHANNEL)
  const [webhookLoading, setWebhookLoading] = useState(false)
  const [webhookTesting, setWebhookTesting] = useState(false)

  const loadData = async () => {
    setLoading(true)
    setError(null)
    
    try {
      const [dataRes, usbRes, airplaneModeRes, webhookRes] = await Promise.all([
        api.getDataStatus(),
        api.getUsbMode(),
        api.getAirplaneMode(),
        api.getWebhookConfig(),
      ])
      
      if (dataRes.data) setDataStatus(dataRes.data.active)
      if (usbRes.data) {
        setUsbMode(usbRes.data)
        setSelectedUsbMode(usbRes.data.current_mode || 1)
      }
      if (airplaneModeRes.data) setAirplaneMode(airplaneModeRes.data)
      if (webhookRes.data) setNotificationChannel(webhookRes.data)

      // 加载健康检查
      await checkHealth()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setLoading(false)
    }
  }

  // 健康检查
  const checkHealth = async () => {
    setHealthLoading(true)
    try {
      const response = await api.health()
      setHealthStatus({
        status: response.status,
        timestamp: new Date().toISOString(),
      })
    } catch {
      setHealthStatus({
        status: 'error',
        timestamp: new Date().toISOString(),
      })
    } finally {
      setHealthLoading(false)
    }
  }

  useEffect(() => {
    void loadData()
    // 每30秒自动检查健康状态
    const interval = setInterval(() => {
      void checkHealth()
    }, 30000)
    return () => clearInterval(interval)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const handleAccordionChange = (panel: string) => (_event: React.SyntheticEvent, isExpanded: boolean) => {
    setExpanded(isExpanded ? panel : false)
  }

  const handleDataToggle = () => {
    void toggleDataConnection()
  }

  const toggleDataConnection = async () => {
    try {
      setError(null)
      setSuccess(null)
      const newStatus = !dataStatus
      await api.setDataStatus(newStatus)
      setDataStatus(newStatus)
      setSuccess(`数据连接已${newStatus ? '启用' : '禁用'}`)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  const handleAirplaneModeToggle = () => {
    void toggleAirplaneMode()
  }

  const toggleAirplaneMode = async () => {
    try {
      setError(null)
      setSuccess(null)
      setAirplaneSwitching(true)
      const newEnabled = !airplaneMode?.enabled
      const response = await api.setAirplaneMode(newEnabled)
      if (response.data) {
        setAirplaneMode(response.data)
        setSuccess(`飞行模式已${newEnabled ? '开启' : '关闭'}`)
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setAirplaneSwitching(false)
    }
  }

  const handleUsbModeApply = () => {
    if (useHotSwitch) {
      void applyUsbModeHot()
    } else {
    void applyUsbMode()
    }
  }

  const applyUsbMode = async () => {
    try {
      setError(null)
      setSuccess(null)
      await api.setUsbMode(selectedUsbMode, usbModePermanent)
      const modeType = usbModePermanent ? '永久' : '临时'
      setSuccess(`USB 模式已设置为 ${getModeNameByValue(selectedUsbMode)} (${modeType})，请重启设备后生效`)
      // 刷新数据
      setTimeout(() => { void loadData() }, 1000)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  // USB 热切换
  const applyUsbModeHot = async () => {
    try {
      setError(null)
      setSuccess(null)
      setHotSwitching(true)
      await api.setUsbModeAdvance(selectedUsbMode)
      setSuccess(`USB 模式已热切换为 ${getModeNameByValue(selectedUsbMode)}（立即生效）`)
      // 刷新数据
      setTimeout(() => { void loadData() }, 2000)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setHotSwitching(false)
    }
  }

  const handleReboot = () => {
    void rebootSystem()
  }

  const rebootSystem = async () => {
    try {
      setError(null)
      setSuccess(null)
      setRebooting(true)
      await api.systemReboot(3)
      setSuccess('系统将在 3 秒后重启...')
    } catch (err) {
      setRebooting(false)
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  const getModeNameByValue = (mode: number) => {
    switch (mode) {
      case 1: return 'CDC-NCM'
      case 2: return 'CDC-ECM'
      case 3: return 'RNDIS'
      default: return 'Unknown'
    }
  }

  // 通知渠道保存
  const handleSaveNotification = async () => {
    setWebhookLoading(true)
    setError(null)
    try {
      const response = await api.setWebhookConfig(notificationChannel)
      if (response.status === 'ok') {
        setSuccess('通知配置已保存')
      } else {
        setError(response.message)
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setWebhookLoading(false)
    }
  }

  const handleTestNotification = async () => {
    setWebhookTesting(true)
    setError(null)
    try {
      const response = await api.testWebhook()
      if (response.status === 'ok' && response.data) {
        if (response.data.success) {
          setSuccess(response.data.message)
        } else {
          setError(response.data.message)
        }
      } else {
        setError(response.message)
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setWebhookTesting(false)
    }
  }

  if (loading) {
    return (
      <Box display="flex" justifyContent="center" alignItems="center" minHeight="60vh">
        <CircularProgress />
      </Box>
    )
  }

  return (
    <Box>
      {/* 页面标题 */}
      <Box mb={3}>
        <Typography variant="h4" gutterBottom fontWeight={600}>
          系统配置
        </Typography>
        <Typography variant="body2" color="text.secondary">
          管理设备连接、USB 模式和其他系统参数
        </Typography>
      </Box>

      {/* 错误和成功提示 Snackbar */}
      <ErrorSnackbar error={error} onClose={() => setError(null)} />
      {success && (
        <Snackbar
          open={true}
          autoHideDuration={3000}
          onClose={() => setSuccess(null)}
          anchorOrigin={{ vertical: 'top', horizontal: 'center' }}
        >
          <Alert severity="success" variant="filled" onClose={() => setSuccess(null)}>
            {success}
          </Alert>
        </Snackbar>
      )}

      {/* 健康检查状态卡片 */}
      <Grid container spacing={3} sx={{ mb: 3 }}>
        <Grid size={{ xs: 12, md: 6 }}>
          <Card>
            <CardHeader
              avatar={<HealthAndSafety color="primary" />}
              title="系统健康检查"
              titleTypographyProps={{ variant: 'h6', fontWeight: 600 }}
              action={
                <Button
                  size="small"
                  onClick={() => void checkHealth()}
                  disabled={healthLoading}
                  startIcon={healthLoading ? <CircularProgress size={16} /> : undefined}
                >
                  刷新
                </Button>
              }
            />
            <CardContent>
              {healthLoading && !healthStatus ? (
                <LinearProgress />
              ) : (
                <Box display="flex" alignItems="center" gap={2}>
                  {healthStatus?.status === 'ok' ? (
                    <CheckCircle sx={{ fontSize: 48, color: 'success.main' }} />
                  ) : (
                    <ErrorIcon sx={{ fontSize: 48, color: 'error.main' }} />
                  )}
                  <Box>
                    <Typography variant="h6" fontWeight={600}>
                      {healthStatus?.status === 'ok' ? '系统正常' : '系统异常'}
                    </Typography>
                    <Typography variant="body2" color="text.secondary">
                      后端服务: <Chip
                        label={healthStatus?.status === 'ok' ? '运行中' : '异常'}
                        size="small"
                        color={healthStatus?.status === 'ok' ? 'success' : 'error'}
                      />
                    </Typography>
                    {healthStatus?.timestamp && (
                      <Typography variant="caption" color="text.secondary">
                        上次检查: {new Date(healthStatus.timestamp).toLocaleTimeString()}
                      </Typography>
                    )}
                  </Box>
                </Box>
              )}
            </CardContent>
          </Card>
        </Grid>

        <Grid size={{ xs: 12, md: 6 }}>
          <Card>
            <CardHeader
              avatar={<Usb color="primary" />}
              title="当前 USB 模式"
              titleTypographyProps={{ variant: 'h6', fontWeight: 600 }}
            />
            <CardContent>
              <Box display="flex" alignItems="center" gap={2}>
                <Chip
                  label={usbMode?.current_mode_name || 'N/A'}
                  color="primary"
                  sx={{ fontSize: '1.1rem', height: 40, px: 2 }}
                />
                <Box>
                  <Typography variant="body2" color="text.secondary">
                    模式代码: {usbMode?.current_mode || 'N/A'}
                  </Typography>
                  {usbMode?.temporary_mode && (
                    <Typography variant="caption" color="warning.main">
                      待重启后切换到: {getModeNameByValue(usbMode.temporary_mode)}
                    </Typography>
                  )}
                </Box>
              </Box>
            </CardContent>
          </Card>
        </Grid>
      </Grid>

      {/* 配置面板 */}
      <Box>
        {/* 数据连接配置 */}
        <Accordion
          expanded={expanded === 'dataConnection'}
          onChange={handleAccordionChange('dataConnection')}
        >
          <AccordionSummary expandIcon={<ExpandMore />}>
            <Box display="flex" alignItems="center" gap={1} width="100%">
              <Wifi color="primary" />
              <Typography fontWeight={600}>数据连接配置</Typography>
              <Box flexGrow={1} />
              <Chip
                label={dataStatus ? '已启用' : '已禁用'}
                color={dataStatus ? 'success' : 'default'}
                size="small"
                onClick={(e: MouseEvent) => e.stopPropagation()}
              />
            </Box>
          </AccordionSummary>
          <AccordionDetails>
            <Typography variant="body2" color="text.secondary" paragraph>
              控制设备的数据连接状态。禁用后设备将断开移动网络连接。
            </Typography>
            
            <Divider sx={{ my: 2 }} />
            
            <FormControlLabel
              control={
                <Switch
                  checked={dataStatus}
                  onChange={handleDataToggle}
                  color="primary"
                />
              }
              label={
                <Box>
                  <Typography variant="body1" fontWeight={600}>
                    {dataStatus ? '数据连接已启用' : '数据连接已禁用'}
                  </Typography>
                  <Typography variant="caption" color="text.secondary">
                    立即{dataStatus ? '断开' : '启用'}移动数据连接
                  </Typography>
                </Box>
              }
            />

            <Alert severity="info" sx={{ mt: 2 }}>
              提示：禁用数据连接将中断所有使用移动网络的应用和服务
            </Alert>
          </AccordionDetails>
        </Accordion>

        {/* 飞行模式配置 */}
        <Accordion
          expanded={expanded === 'airplaneMode'}
          onChange={handleAccordionChange('airplaneMode')}
        >
          <AccordionSummary expandIcon={<ExpandMore />}>
            <Box display="flex" alignItems="center" gap={1} width="100%">
              <FlightTakeoff color={airplaneMode?.enabled ? 'warning' : 'primary'} />
              <Typography fontWeight={600}>飞行模式</Typography>
              <Box flexGrow={1} />
              <Chip
                label={airplaneMode?.enabled ? '已开启' : '已关闭'}
                color={airplaneMode?.enabled ? 'warning' : 'default'}
                size="small"
                onClick={(e: MouseEvent) => e.stopPropagation()}
              />
            </Box>
          </AccordionSummary>
          <AccordionDetails>
            <Typography variant="body2" color="text.secondary" paragraph>
              开启飞行模式将关闭射频，设备将无法连接移动网络。这不会影响 USB 连接。
            </Typography>
            
            <Divider sx={{ my: 2 }} />
            
            <FormControlLabel
              control={
                <Switch
                  checked={airplaneMode?.enabled || false}
                  onChange={handleAirplaneModeToggle}
                  disabled={airplaneSwitching}
                  color="warning"
                />
              }
              label={
                <Box display="flex" alignItems="center" gap={1}>
                  {airplaneSwitching && <CircularProgress size={16} />}
                  <Box>
                    <Typography variant="body1" fontWeight={600}>
                      {airplaneMode?.enabled ? '飞行模式已开启' : '飞行模式已关闭'}
                    </Typography>
                    <Typography variant="caption" color="text.secondary">
                      {airplaneMode?.enabled ? '射频已关闭，无法连接网络' : '射频正常工作'}
                    </Typography>
                  </Box>
                </Box>
              }
            />

            <Box mt={2} p={2} sx={{ bgcolor: 'action.hover', borderRadius: 1 }}>
              <Typography variant="body2" color="text.secondary" gutterBottom>
                <strong>当前状态详情</strong>
              </Typography>
              <Box display="flex" gap={2} flexWrap="wrap">
                <Chip 
                  label={`Modem 电源: ${airplaneMode?.powered ? '开启' : '关闭'}`}
                  size="small"
                  color={airplaneMode?.powered ? 'success' : 'default'}
                  variant="outlined"
                />
                <Chip 
                  label={`射频: ${airplaneMode?.online ? '在线' : '离线'}`}
                  size="small"
                  color={airplaneMode?.online ? 'success' : 'error'}
                  variant="outlined"
                />
              </Box>
            </Box>

            <Alert severity="warning" sx={{ mt: 2 }}>
              注意：飞行模式通过设置 Modem 的 Online 属性来控制射频，与手机的飞行模式效果相同。
            </Alert>
          </AccordionDetails>
        </Accordion>

        {/* USB 配置 */}
        <Accordion
          expanded={expanded === 'usbConfig'}
          onChange={handleAccordionChange('usbConfig')}
        >
          <AccordionSummary expandIcon={<ExpandMore />}>
            <Box display="flex" alignItems="center" gap={1} width="100%">
              <Usb color="primary" />
              <Typography fontWeight={600}>USB 模式配置</Typography>
              <Box flexGrow={1} />
              <Chip
                label={usbMode?.current_mode_name || 'N/A'}
                color="primary"
                size="small"
                onClick={(e: MouseEvent) => e.stopPropagation()}
              />
            </Box>
          </AccordionSummary>
          <AccordionDetails>
            <Typography variant="body2" color="text.secondary" paragraph>
              选择 USB 网络模式。不同模式在不同操作系统上的兼容性和性能各有差异。
            </Typography>
            
            <Divider sx={{ my: 2 }} />
            
            <FormControl component="fieldset" fullWidth>
              <FormLabel component="legend">USB 网络模式</FormLabel>
              <RadioGroup
                value={selectedUsbMode}
                onChange={(e) => setSelectedUsbMode(Number(e.target.value))}
              >
                <FormControlLabel
                  value={1}
                  control={<Radio />}
                  label={
                    <Box>
                      <Typography variant="body1">CDC-NCM (推荐)</Typography>
                      <Typography variant="caption" color="text.secondary">
                        网络控制模型 - 性能最好，支持 Linux/macOS
                      </Typography>
                    </Box>
                  }
                />
                <FormControlLabel
                  value={2}
                  control={<Radio />}
                  label={
                    <Box>
                      <Typography variant="body1">CDC-ECM</Typography>
                      <Typography variant="caption" color="text.secondary">
                        以太网控制模型 - 兼容性好，适用于旧系统
                      </Typography>
                    </Box>
                  }
                />
                <FormControlLabel
                  value={3}
                  control={<Radio />}
                  label={
                    <Box>
                      <Typography variant="body1">RNDIS</Typography>
                      <Typography variant="caption" color="text.secondary">
                        远程网络驱动接口 - Windows 专用模式
                      </Typography>
                    </Box>
                  }
                />
              </RadioGroup>
            </FormControl>

            <Divider sx={{ my: 2 }} />

            {/* USB 热切换选项 */}
            <Box sx={{ mb: 2, p: 2, bgcolor: useHotSwitch ? 'warning.light' : 'action.hover', borderRadius: 1 }}>
              <FormControlLabel
                control={
                  <Switch
                    checked={useHotSwitch}
                    onChange={(e: ChangeEvent<HTMLInputElement>) => setUseHotSwitch(e.target.checked)}
                    color="warning"
                  />
                }
                label={
                  <Box display="flex" alignItems="center" gap={1}>
                    <FlashOn color={useHotSwitch ? 'warning' : 'disabled'} />
                    <Box>
                      <Typography variant="body1" fontWeight={600}>
                        热切换模式(开发中...请勿使用)
                      </Typography>
                      <Typography variant="caption" color="text.secondary">
                        立即切换 USB 模式，无需重启（可能导致短暂断连）
                      </Typography>
                    </Box>
                  </Box>
                }
              />
            </Box>

            {!useHotSwitch && (
              <FormControl component="fieldset" fullWidth sx={{ mb: 2 }}>
              <FormLabel component="legend">配置模式</FormLabel>
              <RadioGroup
                value={usbModePermanent ? 'permanent' : 'temporary'}
                onChange={(e) => setUsbModePermanent(e.target.value === 'permanent')}
              >
                <FormControlLabel
                  value="temporary"
                  control={<Radio />}
                  label={
                    <Box>
                      <Typography variant="body1">临时模式（推荐）</Typography>
                      <Typography variant="caption" color="text.secondary">
                        系统启动时生效一次，然后自动删除配置
                      </Typography>
                    </Box>
                  }
                />
                <FormControlLabel
                  value="permanent"
                  control={<Radio />}
                  label={
                    <Box>
                      <Typography variant="body1">永久模式</Typography>
                      <Typography variant="caption" color="text.secondary">
                        每次系统启动都使用此配置
                      </Typography>
                    </Box>
                  }
                />
              </RadioGroup>
            </FormControl>
            )}

            <Box mt={2} display="flex" gap={2}>
              <Button
                variant="contained"
                fullWidth
                color={useHotSwitch ? 'warning' : 'primary'}
                onClick={handleUsbModeApply}
                disabled={hotSwitching || (selectedUsbMode === usbMode?.current_mode && !useHotSwitch)}
                startIcon={hotSwitching ? <CircularProgress size={20} /> : (useHotSwitch ? <FlashOn /> : undefined)}
              >
                {hotSwitching ? '切换中...' : (useHotSwitch ? '立即热切换' : '保存配置')}
              </Button>
              {!useHotSwitch && (
              <Button
                variant="outlined"
                color="error"
                onClick={handleReboot}
                disabled={rebooting}
                startIcon={rebooting ? <CircularProgress size={20} /> : undefined}
              >
                {rebooting ? '重启中...' : '立即重启'}
              </Button>
              )}
            </Box>

            <Alert severity={useHotSwitch ? 'warning' : 'info'} sx={{ mt: 2 }}>
              <Typography variant="body2" fontWeight={600} gutterBottom>
                {useHotSwitch ? '热切换模式注意事项' : '重要提示'}
              </Typography>
              <Typography variant="body2">
                {useHotSwitch ? (
                  <>
                    - 热切换会立即生效，可能导致网络短暂中断<br/>
                    - 如果切换失败，请使用传统模式并重启设备<br/>
                    - 当前模式：{usbMode?.current_mode_name || 'N/A'}
                  </>
                ) : (
                  <>
                - USB 模式配置需要重启设备后才能生效<br/>
                - 当前硬件运行模式：{usbMode?.current_mode_name || 'N/A'}<br/>
                {usbMode?.temporary_mode && `- 临时配置：${getModeNameByValue(usbMode.temporary_mode)}`}<br/>
                {usbMode?.permanent_mode && `- 永久配置：${getModeNameByValue(usbMode.permanent_mode)}`}
                  </>
                )}
              </Typography>
            </Alert>
          </AccordionDetails>
        </Accordion>

        {/* ========== 通知渠道配置 ========== */}
        <Accordion
          expanded={expanded === 'webhook'}
          onChange={handleAccordionChange('webhook')}
        >
          <AccordionSummary expandIcon={<ExpandMore />}>
            <Box display="flex" alignItems="center" gap={1} width="100%">
              <Webhook color={notificationChannel.channel !== 'none' ? 'success' : 'primary'} />
              <Typography fontWeight={600}>通知渠道</Typography>
              <Box flexGrow={1} />
              <Chip
                label={CHANNEL_OPTIONS.find(o => o.value === notificationChannel.channel)?.label ?? '未配置'}
                color={notificationChannel.channel !== 'none' ? 'success' : 'default'}
                size="small"
                onClick={(e: MouseEvent) => e.stopPropagation()}
              />
            </Box>
          </AccordionSummary>
          <AccordionDetails>
            <Typography variant="body2" color="text.secondary" paragraph>
              选择一种通知渠道来推送短信和来电事件。同一时间只能启用一个渠道。
            </Typography>

            <Divider sx={{ my: 2 }} />

            {/* 渠道选择 */}
            <FormControl component="fieldset" fullWidth>
              <FormLabel component="legend">通知渠道</FormLabel>
              <RadioGroup
                value={notificationChannel.channel}
                onChange={(e) => setNotificationChannel(prev => ({
                  ...prev,
                  channel: e.target.value as ChannelType
                }))}
              >
                <Grid container spacing={1}>
                  {CHANNEL_OPTIONS.map(opt => (
                    <Grid size={{ xs: 12, sm: 6, md: 4 }} key={opt.value}>
                      <Box
                        onClick={() => setNotificationChannel(prev => ({
                          ...prev,
                          channel: opt.value
                        }))}
                        sx={{
                          p: 1.5,
                          border: '1px solid',
                          borderColor: notificationChannel.channel === opt.value ? 'primary.main' : 'divider',
                          borderRadius: 2,
                          cursor: 'pointer',
                          bgcolor: notificationChannel.channel === opt.value ? 'primary.light' : 'background.paper',
                          '&:hover': { borderColor: 'primary.main' },
                          transition: 'all 0.2s',
                        }}
                      >
                        <Box display="flex" alignItems="center" gap={1}>
                          <Radio value={opt.value} sx={{ p: 0 }} />
                          <Box>
                            <Typography variant="body2" fontWeight={600}>{opt.label}</Typography>
                            <Typography variant="caption" color="text.secondary">{opt.desc}</Typography>
                          </Box>
                        </Box>
                      </Box>
                    </Grid>
                  ))}
                </Grid>
              </RadioGroup>
            </FormControl>

            <Divider sx={{ my: 2 }} />

            {/* 动态渲染各渠道配置表单 */}
            {notificationChannel.channel === 'dingtalk' && (
              <DingtalkForm
                config={notificationChannel.dingtalk}
                onChange={c => setNotificationChannel(prev => ({ ...prev, dingtalk: c }))}
              />
            )}
            {notificationChannel.channel === 'feishu' && (
              <FeishuForm
                config={notificationChannel.feishu}
                onChange={c => setNotificationChannel(prev => ({ ...prev, feishu: c }))}
              />
            )}
            {notificationChannel.channel === 'wecom' && (
              <WecomForm
                config={notificationChannel.wecom}
                onChange={c => setNotificationChannel(prev => ({ ...prev, wecom: c }))}
              />
            )}
            {notificationChannel.channel === 'email' && (
              <EmailForm
                config={notificationChannel.email}
                onChange={c => setNotificationChannel(prev => ({ ...prev, email: c }))}
              />
            )}
            {notificationChannel.channel === 'bark' && (
              <BarkForm
                config={notificationChannel.bark}
                onChange={c => setNotificationChannel(prev => ({ ...prev, bark: c }))}
              />
            )}
            {notificationChannel.channel === 'none' && (
              <Alert severity="info">已关闭通知渠道，不推送任何消息。</Alert>
            )}

            <Divider sx={{ my: 2 }} />

            {/* 转发选项（仅在非 none 时显示） */}
            {notificationChannel.channel !== 'none' && (
              <Box display="flex" gap={3} mb={2}>
                <FormControlLabel
                  control={
                    <Switch
                      checked={notificationChannel.forward_sms}
                      onChange={(e) => setNotificationChannel(prev => ({
                        ...prev,
                        forward_sms: e.target.checked
                      }))}
                    />
                  }
                  label="转发短信"
                />
                <FormControlLabel
                  control={
                    <Switch
                      checked={notificationChannel.forward_calls}
                      onChange={(e) => setNotificationChannel(prev => ({
                        ...prev,
                        forward_calls: e.target.checked
                      }))}
                    />
                  }
                  label="转发来电"
                />
              </Box>
            )}

            {/* 模板变量提示 */}
            {notificationChannel.channel !== 'none' && notificationChannel.channel !== 'email' && notificationChannel.channel !== 'bark' && (
              <Alert severity="info" sx={{ mb: 2 }}>
                <Typography variant="body2">
                  <strong>支持的模板变量：</strong><br/>
                  短信: <code>{'{{phone_number}}'}</code>, <code>{'{{content}}'}</code>, <code>{'{{timestamp}}'}</code>, <code>{'{{direction}}'}</code><br/>
                  通话: <code>{'{{phone_number}}'}</code>, <code>{'{{direction_cn}}'}</code>, <code>{'{{duration}}'}</code>, <code>{'{{start_time}}'}</code>, <code>{'{{answered}}'}</code>
                </Typography>
              </Alert>
            )}

            {/* 操作按钮 */}
            <Box display="flex" gap={2}>
              <Button
                variant="contained"
                fullWidth
                onClick={() => void handleSaveNotification()}
                disabled={webhookLoading}
                startIcon={webhookLoading ? <CircularProgress size={20} /> : undefined}
              >
                {webhookLoading ? '保存中...' : '保存配置'}
              </Button>
              <Button
                variant="outlined"
                onClick={() => void handleTestNotification()}
                disabled={webhookTesting || notificationChannel.channel === 'none'}
                startIcon={webhookTesting ? <CircularProgress size={20} /> : <PlayArrow />}
              >
                {webhookTesting ? '测试中...' : '测试'}
              </Button>
            </Box>
          </AccordionDetails>
        </Accordion>
      </Box>
    </Box>
  )
}
