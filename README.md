# lightsftp
A light sftp server/client

# LightSFTP

## 介绍

一个 Rust 实现的 Linux Light SFTP Server

## 支持的功能

- 大部分的 FTP 命令
- 支持主被动传输模式
- 支持用户自定义配置信息
- 支持指定被动模式下数据端口的范围，考虑到了主机配置有防火墙的情况
- 支持文件上传/下载的断点续传
- 支持限速功能，防止服务过多占用带宽资源
- 限流，防 DDOS 攻击
- Reactor 异步方式实现

## 快速运行此代码

### 可执行程序

### 源码编译

#### 环境依赖

#### 编译步骤
